use std::marker::PhantomData;

use futures::Poll;

use super::cell::Cell;
use super::service::Service;

/// Service that allows to turn non-clone service to a service with `Clone` impl
pub struct CloneableService<S: Service<R> + 'static, R> {
    service: Cell<S>,
    _t: PhantomData<R>,
}

impl<S: Service<R> + 'static, R> CloneableService<S, R> {
    pub fn new(service: S) -> Self {
        Self {
            service: Cell::new(service),
            _t: PhantomData,
        }
    }
}

impl<S: Service<R> + 'static, R> Clone for CloneableService<S, R> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            _t: PhantomData,
        }
    }
}

impl<S: Service<R> + 'static, R> Service<R> for CloneableService<S, R> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.borrow_mut().poll_ready()
    }

    fn call(&mut self, req: R) -> Self::Future {
        self.service.borrow_mut().call(req)
    }
}
