use std::cell::UnsafeCell;
use std::rc::Rc;

use actix_service::Service;
use futures::Poll;

#[doc(hidden)]
/// Service that allows to turn non-clone service to a service with `Clone` impl
pub(crate) struct CloneableService<T>(Rc<UnsafeCell<T>>);

impl<T> CloneableService<T> {
    pub(crate) fn new(service: T) -> Self
    where
        T: Service,
    {
        Self(Rc::new(UnsafeCell::new(service)))
    }
}

impl<T> Clone for CloneableService<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Service for CloneableService<T>
where
    T: Service,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        unsafe { &mut *self.0.as_ref().get() }.poll_ready()
    }

    fn call(&mut self, req: T::Request) -> Self::Future {
        unsafe { &mut *self.0.as_ref().get() }.call(req)
    }
}
