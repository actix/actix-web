use std::marker::PhantomData;
use std::rc::Rc;

use futures::Poll;

use super::cell::Cell;
use super::service::Service;

/// Service that allows to turn non-clone service to a service with `Clone` impl
pub struct CloneableService<T: 'static> {
    service: Cell<T>,
    _t: PhantomData<Rc<()>>,
}

impl<T: 'static> CloneableService<T> {
    pub fn new<Request>(service: T) -> Self
    where
        T: Service<Request>,
    {
        Self {
            service: Cell::new(service),
            _t: PhantomData,
        }
    }
}

impl<T: 'static> Clone for CloneableService<T> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            _t: PhantomData,
        }
    }
}

impl<T: 'static, Request> Service<Request> for CloneableService<T>
where
    T: Service<Request>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.borrow_mut().poll_ready()
    }

    fn call(&mut self, req: Request) -> Self::Future {
        self.service.borrow_mut().call(req)
    }
}
