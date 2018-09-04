use std::marker::PhantomData;

use futures::{Future, Poll};
use tower_service::Service;

pub struct FromErr<A, E>
where
    A: Service,
{
    service: A,
    f: PhantomData<E>,
}

impl<A: Service, E: From<A::Error>> FromErr<A, E> {
    pub(crate) fn new(service: A) -> Self {
        FromErr {
            service,
            f: PhantomData,
        }
    }
}

impl<A, E> Service for FromErr<A, E>
where
    A: Service,
    E: From<A::Error>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;
    type Future = FromErrFuture<A, E>;

    fn poll_ready(&mut self) -> Poll<(), E> {
        Ok(self.service.poll_ready().map_err(E::from)?)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        FromErrFuture {
            fut: self.service.call(req),
            f: PhantomData,
        }
    }
}

pub struct FromErrFuture<A: Service, E> {
    fut: A::Future,
    f: PhantomData<E>,
}

impl<A, E> Future for FromErrFuture<A, E>
where
    A: Service,
    E: From<A::Error>,
{
    type Item = A::Response;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(E::from)
    }
}
