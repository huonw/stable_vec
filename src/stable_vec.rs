#![crate_name="stable_vec"]
#![crate_type="lib"]

#![feature(macro_rules, phase, unsafe_destructor)]

#[cfg(test)] extern crate test;
#[phase(plugin, link)] extern crate log;

use std::kinds::marker;
use std::{mem, ptr};

macro_rules! debug_assert {
    ($cond: expr) => {
        debug_assert!($cond, "assertion {} failed", stringify!($cond))
    };
    ($cond: expr, $($msg:tt)*) => {
        if !cfg!(ndebug) {
            if !$cond { fail!($($msg)*) }
        }
    }
}

struct Entry<T> {
    value: T,
    base_ptr: *mut *mut Entry<T>
}
pub struct StableVec<T> {
    /// Invariants:
    /// - empty, or
    /// - the last element is a dummy entry to enable nicer iterators.
    vec: Vec<Box<Entry<T>>>
}

impl<T> StableVec<T> {
    pub fn new() -> StableVec<T> {
        StableVec { vec: Vec::new() }
    }
    pub fn with_capacity(n: uint) -> StableVec<T> {
        // n + 1 for the dummy
        StableVec { vec: Vec::with_capacity(n + 1) }
    }

    pub fn reserve(&mut self, n: uint) {
        // n + 1 for the dummy
        // FIXME: this pointer checking is quite fragile
        let p = self.vec.as_ptr();
        self.vec.reserve(n + 1);
        if p != self.vec.as_ptr() {
            unsafe {self.fix_from(0)}
        }
    }
    pub fn reserve_additional(&mut self, n: uint) {
        let p = self.vec.as_ptr();
        self.vec.reserve_additional(n);
        if p != self.vec.as_ptr() {
            unsafe {self.fix_from(0)}
        }
    }

    pub fn handle<'a>(&'a mut self) -> Handle<'a, T> {
        Handle {
            lifetime: marker::ContravariantLifetime,
            sv: self
        }
    }

    pub fn get<'a>(&'a self, index: uint) -> &'a T {
        if index >= self.len() {
            fail!("StableVec.get: index {} greater than length {}", index, self.len())
        } else {
            &self.vec[index].value
        }
    }

    fn iter<'a>(&'a self) -> Items<'a, T> {
        // if we're empty then make an empty iterator (we might not
        // have the dummy node to substitute): unfortunately this
        // means that any updates to the vec are completely ignored,
        // but that's true even using the else branch (if we
        // guaranteed the existance of the dummy).
        if self.vec.len() == 0 {
            Items {
                lifetime: marker::ContravariantLifetime,
                start: ptr::null(),
                end: ptr::null()
            }
        } else {
            Items {
                lifetime: marker::ContravariantLifetime,
                start: &*self.vec[0],
                end: &**self.vec.last().unwrap()
            }
        }
    }

    // FIXME: this should be able to reuse a previous dummy i.e. each
    // StableVec should need to allocate a dummy exactly once.
    unsafe fn add_dummy(&mut self) {
        self.push_single(mem::zeroed());
    }
    unsafe fn remove_dummy(&mut self) {
        // kill the dummy end one
        // FIXME: leaks memory.
        mem::forget(self.vec.pop())
    }
    unsafe fn fix_from(&mut self, i: uint) {
        for elem in self.vec.mut_slice_from(i).mut_iter() {
            let p = elem as *mut _ as *mut _;
            elem.base_ptr = p;
        }
    }

    /// FIXME: this should only need to do one `fix_from` call, rather
    /// than one for `x` and one for `add_dummy`
    fn push<'a>(&'a mut self, x: T) -> &'a mut T {
        unsafe {self.remove_dummy()}
        let p = self.push_single(x) as *mut _;
        unsafe {
            self.add_dummy();
            &mut *p
        }
    }
    /// Push an element directly onto the vector, making no adjustments.
    fn push_nofix<'a>(&'a mut self, value: T) {
        self.vec.push(box Entry { value: value, base_ptr: ptr::mut_null() });
    }
    /// Push a single value directly onto the end of the vector,
    /// *without* adjusting the dummy.
    fn push_single<'a>(&'a mut self, value: T) -> &'a mut T {
        let start_ptr = self.vec.as_ptr();
        let i = self.vec.len();
        self.push_nofix(value);
        let end_ptr = self.vec.as_ptr();

        let this_elem = self.vec.get_mut(i) as *mut _;
        if start_ptr == end_ptr {
            let elem = unsafe {&mut *this_elem};
            let p = elem as *mut _ as *mut _;
            elem.base_ptr = p;

            &mut elem.value
        } else {
            unsafe {
                self.fix_from(0);

                &mut (*this_elem).value
            }
        }
    }

    fn insert<'a>(&'a mut self, index: uint, value: T) -> &'a mut T {
        if index >= self.len() {
            fail!("StableVec.insert: index {} > length {}", index, self.len())
        }

        let start_ptr = self.vec.as_ptr();
        self.vec.insert(index, box Entry { value: value, base_ptr: ptr::mut_null() });
        let end_ptr = self.vec.as_ptr();

        let n = if start_ptr == end_ptr { index } else { 0 };
        unsafe {self.fix_from(n)}

        &mut self.vec.get_mut(index).value
    }
}

#[unsafe_destructor]
impl<T> Drop for StableVec<T> {
    fn drop(&mut self) {
        unsafe {self.remove_dummy()}
    }
}

impl<T> Collection for StableVec<T> {
    fn len(&self) -> uint {
        // we can be empty in two ways: self.vec being completely
        // empty & self.vec just containing the dummy (and any
        // non-zero length vector contains the dummy for sure).
        if self.vec.len() == 0 {
            0
        } else {
            self.vec.len() - 1
        }
    }
}

impl<T> FromIterator<T> for StableVec<T> {
    fn from_iter<It: Iterator<T>>(it: It) -> StableVec<T> {
        let (n, _) = it.size_hint();
        let mut sv = StableVec::with_capacity(n);
        sv.extend(it);
        sv
    }
}

impl<T> Extendable<T> for StableVec<T> {
    fn extend<It: Iterator<T>>(&mut self, mut it: It) {
        let (n, _) = it.size_hint();

        self.reserve_additional(n);

        // FIXME: this could probably be more efficient, but needs to
        // ensure the dummy node is handled correctly.
        for elem in it {
            self.push(elem);
        }
    }
}

// fixme: I don't think this works "properly" when created from an
// empty vector (start == end always in that case). Is this concerning?
pub struct Items<'a, T> {
    lifetime: marker::ContravariantLifetime<'a>,
    start: *const Entry<T>,
    end: *const Entry<T>,
}

impl<'a, T> Iterator<&'a T> for Items<'a, T> {
    fn next(&mut self) -> Option<&'a T> {
        if self.start == self.end {
            None
        } else {
            let p = self.start;
            unsafe {
                self.start = *(*p).base_ptr.offset(1) as *const _;
                Some(&(*p).value)
            }
        }
    }
}
impl<'a, T> DoubleEndedIterator<&'a T> for Items<'a, T> {
    fn next_back(&mut self) -> Option<&'a T> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                self.end = *(*self.end).base_ptr.offset(-1) as *const _;
                Some(&(*self.end).value)
            }
        }
    }
}

pub struct Handle<'a, T> {
    lifetime: marker::ContravariantLifetime<'a>,
    sv: *mut StableVec<T>
}

impl<'a, T> Handle<'a, T> {
    pub fn get(&self, index: uint) -> &'a T {
        unsafe {self.sv().get(index)}
    }

    pub fn push(&mut self, value: T) -> &'a T {
        unsafe {&*self.mut_sv().push(value)}
    }

    pub fn insert(&mut self, index: uint, value: T) -> &'a T {
        unsafe {&*self.mut_sv().insert(index, value)}
    }

    pub fn iter(&self) -> Items<'a, T> {
        unsafe {self.sv().iter()}
    }

    /// Get a mutable reference to the internal StableVec. Be careful
    /// that the returned `&mut` reference doesn't alias with anything.
    unsafe fn mut_sv(&mut self) -> &'a mut StableVec<T> {
        &mut *self.sv
    }

    /// Get a reference to the internal StableVec. Be careful that the
    /// created `&` reference doesn't have any modifications occur
    /// under its feet.
    unsafe fn sv(&self) -> &'a StableVec<T> {
        &*self.sv
    }
}

impl<'a,T> Collection for Handle<'a,T> {
    fn len(&self) -> uint {
        unsafe {(*self.sv).len()}
    }
}
// FIXME: it's annoying that this is required to get Extendable to
// work.
impl<'a,T> FromIterator<T> for Handle<'a,T> {
    fn from_iter<It: Iterator<T>>(_: It) -> Handle<'a, T> {
        fail!("StableVec: called `FromIterator` with a `Handle`")
    }
}

impl<'a,T> Extendable<T> for Handle<'a,T> {
    // FIXME: this is probably incorrect: the iterator could probably
    // capture another handle to the vector, meaning the removal of
    // the dummy can probably cause crashes.
    fn extend<It: Iterator<T>>(&mut self, it: It) {
        unsafe {(*self.sv).extend(it)}
    }
}

#[cfg(test)]
mod tests {
    use super::StableVec;
    use std::{task, io};

    #[test]
    fn push_len() {
        let mut x = StableVec::new();
        assert_eq!(x.len(), 0);
        for i in range(1u, 100) {
            x.push(i);
            assert_eq!(x.len(), i);
        }
    }

    #[test]
    fn handle_get() {
        let mut x = StableVec::new();
        let mut h = x.handle();
        h.extend(range(0u, 10));
        for i in range(0u, 10) {
            assert_eq!(*h.get(i), i)
        }
    }

    #[test]
    fn handle_push_len() {
        let mut x = StableVec::new();
        {
            let mut h = x.handle();
            assert_eq!(h.len(), 0);
            for i in range(1u, 100) {
                h.push(i);
                assert_eq!(h.len(), i);
            }
        }
        assert_eq!(x.len(), 99)
    }

    #[test]
    fn handle_iter() {
        let mut x = StableVec::new();
        x.extend(range(0u, 6));

        let mut it = x.handle().iter().map(|&x| x);
        assert_eq!(it.next(), Some(0));
        assert_eq!(it.next_back(), Some(5));
        assert_eq!(it.next(), Some(1));
        assert_eq!(it.next(), Some(2));
        assert_eq!(it.next_back(), Some(4));
        assert_eq!(it.next_back(), Some(3));
        assert_eq!(it.next(), None);
        assert_eq!(it.next_back(), None);
    }

    #[test]
    fn handle_push_iter() {
        let mut x = StableVec::new();

        let mut h = x.handle();
        for i in range(0u, 10) {
            h.push(i);
        }
        for j in h.iter().take(100) {
            h.push(j + 10);
        }

        for (i, x) in h.iter().enumerate() {
            assert_eq!(*x, i)
        }
    }

    #[test]
    fn handle_insert_iter() {
        let mut x = StableVec::new();

        let mut h = x.handle();
        h.push(1u);
        h.push(2);
        let mut it = h.iter().map(|&x| x);
        h.insert(1, 10);
        h.insert(1, 20);
        assert_eq!(it.next(), Some(1));
        assert_eq!(it.next(), Some(20));
        h.insert(3, 30);
        assert_eq!(it.next(), Some(10));
        assert_eq!(it.next(), Some(30));
        h.insert(2, 40);
        assert_eq!(it.next(), Some(2));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn from_iter() {
        let sv: StableVec<uint> = range(0u, 10).collect();
        for (i, elem) in sv.iter().enumerate() {
            assert_eq!(*elem, i)
        }
    }
    #[test]
    fn extend() {
        let mut sv: StableVec<_> = range(0u, 10).collect();
        sv.extend(range(10u, 20));
        for (i, elem) in sv.iter().enumerate() {
            assert_eq!(*elem, i);
        }
    }

    #[test]
    fn handle_extend() {
        let mut sv = StableVec::new();
        {
            let mut h = sv.handle();
            h.extend(range(0u, 10));
        }
        for (i, elem) in sv.iter().enumerate() {
            assert_eq!(*elem, i);
        }
    }

    #[test]
    fn get_oob() {
        for n in range(0u, 10) {
            for i in range(n, n+10) {
                let sv: StableVec<uint> = range(0, n).collect();
                let res = task::try(proc() {
                    io::stdio::set_stderr(box io::util::NullWriter);
                    sv.get(i);
                });
                assert!(res.is_err());
            }
        }
    }

    // check that a variety of uses of StableVec are failure-safe,
    // i.e. that failure runs each destructor once (and only once)
    #[test]
    fn dtor_fail() {
        static mut DROP_COUNT: u64 = 0;

        struct Counter { x: uint }
        impl Drop for Counter {
            fn drop(&mut self) {
                let bit = 1 << self.x;
                unsafe {
                    assert!(DROP_COUNT & bit == 0)
                    DROP_COUNT |= bit;
                }
            }
        }

        for n in range(0u, 32) {
            macro_rules! e { ($e:expr) => { $e } }
            macro_rules! t {
                ($iter: ident => $($inner:tt)*) => {
                    {
                        unsafe {DROP_COUNT = 0}

                        let ret = task::try(proc() {
                            io::stdio::set_stderr(box io::util::NullWriter);

                            let mut $iter = range(0u, n).map(|x| Counter { x: x })
                                .chain(Some(()).move_iter().map(|_| -> Counter fail!()));
                            e!({$($inner)*});
                        });
                        assert!(ret.is_err())
                        unsafe {
                            assert_eq!(DROP_COUNT, (1 << n) - 1);
                        }

                    }
                }
            }

            t!(iter => let _: StableVec<Counter> = iter.collect(););
            t!(_iter => {
                let mut sv = StableVec::new();
                sv.extend(_iter);
            });
            t!(iter => {
                let mut sv = StableVec::new();
                for x in iter { sv.push(x); }
            });
            t!(iter => {
                let mut sv = StableVec::new();
                let mut h = sv.handle();
                for x in iter { h.push(x); }
            });
        }
    }

    #[test]
    fn extend_handle_in_iterator() {
        // one handle can be used to extend while another is used
        // *inside* the iterator.
        let mut sv = StableVec::new();
        let mut h = sv.handle();
        h.push(1u);
        let g = h;
        h.extend(range(0u, 2).filter_map(|_| g.iter().next().map(|&x|x)));
        assert_eq!(h.len(), 3);
    }
}


#[cfg(test)]
mod benches {
    use super::StableVec;
    use test::Bencher;

    #[bench]
    fn new(b: &mut Bencher) {
        b.iter(|| StableVec::<int>::new());
    }


    #[bench]
    fn push_100(b: &mut Bencher) {
        b.iter(|| {
            let mut sv = StableVec::new();
            {
                let mut h = sv.handle();
                for x in range(0u, 100) { h.push(x); }
            }
            sv
        });
    }
    #[bench]
    fn extend_100(b: &mut Bencher) {
        b.iter(|| {
            let mut sv = StableVec::new();
            {
                let mut h = sv.handle();
                h.extend(range(0u, 100));
            }
            sv
        })
    }
    #[bench]
    fn collect_100(b: &mut Bencher) {
        b.iter(|| {
            range(0i, 100).collect::<StableVec<int>>()
        })
    }


    #[bench]
    fn iter_100(b: &mut Bencher) {
        let mut sv: StableVec<int> = range(0i, 100).collect();
        let h = sv.handle();
        b.iter(|| {
            for i in h.iter() { ::test::black_box(i) }
        })
    }
}
