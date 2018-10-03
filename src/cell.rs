//! Custom cell impl

#[cfg(feature = "cell")]
use std::cell::UnsafeCell;
#[cfg(not(feature = "cell"))]
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::rc::Rc;

pub(crate) struct Cell<T> {
    #[cfg(feature = "cell")]
    inner: Rc<UnsafeCell<T>>,
    #[cfg(not(feature = "cell"))]
    inner: Rc<RefCell<T>>,
}

impl<T> Clone for Cell<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Cell<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

#[cfg(feature = "cell")]
impl<T> Cell<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            inner: Rc::new(UnsafeCell::new(inner)),
        }
    }

    pub(crate) fn borrow(&self) -> &T {
        unsafe { &*self.inner.as_ref().get() }
    }

    pub(crate) fn borrow_mut(&self) -> &mut T {
        unsafe { &mut *self.inner.as_ref().get() }
    }
}

#[cfg(not(feature = "cell"))]
impl<T> Cell<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    pub(crate) fn borrow(&self) -> Ref<T> {
        self.inner.borrow()
    }
    pub(crate) fn borrow_mut(&self) -> RefMut<T> {
        self.inner.borrow_mut()
    }
}
