use std::cell::RefCell;
use std::rc::Rc;

use futures::Poll;

use super::service::Service;

/// Service that allows to turn non-clone service to a service with `Clone` impl
pub struct CloneableService<S: Service + 'static> {
    service: Rc<RefCell<S>>,
}

impl<S: Service + 'static> CloneableService<S> {
    pub fn new(service: S) -> Self {
        Self {
            service: Rc::new(RefCell::new(service)),
        }
    }
}

impl<S: Service + 'static> Clone for CloneableService<S> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
        }
    }
}

impl<S: Service + 'static> Service for CloneableService<S> {
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.borrow_mut().poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.service.borrow_mut().call(req)
    }
}
