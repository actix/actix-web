use std::cell::UnsafeCell;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_service::Service;

#[doc(hidden)]
/// Service that allows to turn non-clone service to a service with `Clone` impl
pub(crate) struct CloneableService<T: Service>(Rc<UnsafeCell<T>>);

impl<T: Service> CloneableService<T> {
    pub(crate) fn new(service: T) -> Self {
        Self(Rc::new(UnsafeCell::new(service)))
    }
}

impl<T: Service> Clone for CloneableService<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Service> Service for CloneableService<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        unsafe { &mut *self.0.as_ref().get() }.poll_ready(cx)
    }

    fn call(&mut self, req: T::Request) -> Self::Future {
        unsafe { &mut *self.0.as_ref().get() }.call(req)
    }
}
