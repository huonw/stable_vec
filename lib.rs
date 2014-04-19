#![crate_id="stable_vec"]
#![crate_type="lib"]

#![feature(macro_rules)]

use std::kinds::marker;
use std::{cast, mem, ptr};

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
    vec: Vec<~Entry<T>>
}

impl<T> StableVec<T> {
    pub fn new() -> StableVec<T> {
        let mut sv = StableVec { vec: vec![] };
        unsafe {sv.add_dummy()};
        sv
    }

    pub fn handle<'a>(&'a mut self) -> Handle<'a, T> {
        Handle {
            lifetime: marker::ContravariantLifetime,
            sv: self
        }
    }

    fn iter<'a>(&'a self) -> Items<'a, T> {
        debug_assert!(self.vec.len() >= 1)
        Items {
            lifetime: marker::ContravariantLifetime,
            start: &**self.vec.get(0),
            end: &**self.vec.last().unwrap()
        }
    }

    fn push<'a>(&'a mut self, x: T) -> &'a mut T {
        debug_assert!(self.vec.len() >= 1)
        unsafe {self.remove_dummy()};
        let p = self.push_single(x) as *mut _;
        unsafe {
            self.add_dummy();
            &mut *p
        }
    }

    unsafe fn add_dummy(&mut self) {
        self.push_single(mem::init());
    }
    unsafe fn remove_dummy(&mut self) {
        // kill the dummy end one
        cast::forget(self.vec.pop())
    }
    fn push_single<'a>(&'a mut self, value: T) -> &'a mut T {
        let start_ptr = self.vec.as_ptr();
        let i = self.vec.len();
        self.vec.push(~Entry { value: value, base_ptr: ptr::mut_null() });
        let end_ptr = self.vec.as_ptr();

        if start_ptr == end_ptr {
            let elem = self.vec.get_mut(i);
            let p = elem as *mut _ as *mut _;
            elem.base_ptr = p;

            &mut elem.value
        } else {
            for elem in self.vec.mut_iter() {
                let p = elem as *mut _ as *mut _;
                elem.base_ptr = p;
            }

            &mut self.vec.get_mut(i).value
        }
    }

    fn insert<'a>(&'a mut self, index: uint, value: T) -> &'a mut T {
        if index > self.len() {
            fail!("StableVec.insert: index {} > length {}", index, self.len())
        }

        let start_ptr = self.vec.as_ptr();
        self.vec.insert(index, ~Entry { value: value, base_ptr: ptr::mut_null() });
        let end_ptr = self.vec.as_ptr();

        {
            let v = if start_ptr == end_ptr {
                self.vec.mut_slice_from(index)
            } else {
                self.vec.as_mut_slice()
            };
            for elem in v.mut_iter() {
                let p = elem as *mut _ as *mut _;
                elem.base_ptr = p;
            }
        }

        &mut self.vec.get_mut(index).value
    }
}

impl<T> Container for StableVec<T> {
    fn len(&self) -> uint {
        debug_assert!(self.vec.len() >= 1)
        self.vec.len() - 1
    }
}

// fixme: I don't think this works "properly" when created from an
// empty vector (start == end always in that case). Is this concerning?
pub struct Items<'a, T> {
    lifetime: marker::ContravariantLifetime<'a>,
    start: *Entry<T>,
    end: *Entry<T>,
}

impl<'a, T> Iterator<&'a T> for Items<'a, T> {
    fn next(&mut self) -> Option<&'a T> {
        if self.start == self.end {
            None
        } else {
            let p = self.start;
            unsafe {
                self.start = *(*p).base_ptr.offset(1) as *_;
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
                self.end = *(*self.end).base_ptr.offset(-1) as *_;
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
    pub fn push(&mut self, value: T) -> &'a T {
        unsafe {&*(*self.sv).push(value)}
    }

    pub fn insert(&mut self, index: uint, value: T) -> &'a T {
        unsafe {&*(*self.sv).insert(index, value)}
    }

    pub fn iter(&self) -> Items<'a, T> {
        unsafe {(*self.sv).iter()}
    }
}

impl<'a,T> Container for Handle<'a,T> {
    fn len(&self) -> uint {
        unsafe {(*self.sv).len()}
    }
}


#[unsafe_destructor]
impl<T> Drop for StableVec<T> {
    fn drop(&mut self) {
        unsafe {self.remove_dummy()}
    }
}


#[cfg(test)]
mod tests {
    use super::StableVec;

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
        let mut h = x.handle();
        for i in range(0, 6) {
            h.push(i);
        }

        let mut it = h.iter().map(|&x| x);
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
        h.push(1);
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
}
