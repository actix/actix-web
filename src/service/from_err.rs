use std::marker::PhantomData;

use futures::{Async, Future, Poll};

use super::{NewService, Service};

/// Service for the `from_err` combinator, changing the error type of a service.
///
/// This is created by the `ServiceExt::from_err` method.
pub struct FromErr<A, E> {
    service: A,
    f: PhantomData<E>,
}

impl<A, E> FromErr<A, E> {
    pub(crate) fn new<Request>(service: A) -> Self
    where
        A: Service<Request>,
        E: From<A::Error>,
    {
        FromErr {
            service,
            f: PhantomData,
        }
    }
}

impl<A, E> Clone for FromErr<A, E>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        FromErr {
            service: self.service.clone(),
            f: PhantomData,
        }
    }
}

impl<A, E, Request> Service<Request> for FromErr<A, E>
where
    A: Service<Request>,
    E: From<A::Error>,
{
    type Response = A::Response;
    type Error = E;
    type Future = FromErrFuture<A, E, Request>;

    fn poll_ready(&mut self) -> Poll<(), E> {
        Ok(self.service.poll_ready().map_err(E::from)?)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        FromErrFuture {
            fut: self.service.call(req),
            f: PhantomData,
        }
    }
}

pub struct FromErrFuture<A: Service<Request>, E, Request> {
    fut: A::Future,
    f: PhantomData<E>,
}

impl<A, E, Request> Future for FromErrFuture<A, E, Request>
where
    A: Service<Request>,
    E: From<A::Error>,
{
    type Item = A::Response;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(E::from)
    }
}

/// NewService for the `from_err` combinator, changing the type of a new
/// service's error.
///
/// This is created by the `NewServiceExt::from_err` method.
pub struct FromErrNewService<A, E> {
    a: A,
    e: PhantomData<E>,
}

impl<A, E> FromErrNewService<A, E> {
    /// Create new `FromErr` new service instance
    pub fn new<Request>(a: A) -> Self
    where
        A: NewService<Request>,
        E: From<A::Error>,
    {
        Self { a, e: PhantomData }
    }
}

impl<A, E> Clone for FromErrNewService<A, E>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            e: PhantomData,
        }
    }
}

impl<A, E, Request> NewService<Request> for FromErrNewService<A, E>
where
    A: NewService<Request>,
    E: From<A::Error>,
{
    type Response = A::Response;
    type Error = E;
    type Service = FromErr<A::Service, E>;

    type InitError = A::InitError;
    type Future = FromErrNewServiceFuture<A, E, Request>;

    fn new_service(&self) -> Self::Future {
        FromErrNewServiceFuture {
            fut: self.a.new_service(),
            e: PhantomData,
        }
    }
}

pub struct FromErrNewServiceFuture<A, E, Request>
where
    A: NewService<Request>,
    E: From<A::Error>,
{
    fut: A::Future,
    e: PhantomData<E>,
}

impl<A, E, Request> Future for FromErrNewServiceFuture<A, E, Request>
where
    A: NewService<Request>,
    E: From<A::Error>,
{
    type Item = FromErr<A::Service, E>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(FromErr::new(service)))
        } else {
            Ok(Async::NotReady)
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{err, FutureResult};

    use super::*;
    use service::{IntoNewService, NewServiceExt, Service, ServiceExt};

    struct Srv;
    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = FutureResult<(), ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Err(())
        }

        fn call(&mut self, _: ()) -> Self::Future {
            err(())
        }
    }

    #[derive(Debug, PartialEq)]
    struct Error;

    impl From<()> for Error {
        fn from(_: ()) -> Self {
            Error
        }
    }

    #[test]
    fn test_poll_ready() {
        let mut srv = Srv.from_err::<Error>();
        let res = srv.poll_ready();
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), Error);
    }

    #[test]
    fn test_call() {
        let mut srv = Srv.from_err::<Error>();
        let res = srv.call(()).poll();
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), Error);
    }

    #[test]
    fn test_new_service() {
        let blank = || Ok::<_, ()>(Srv);
        let new_srv = blank.into_new_service().from_err::<Error>();
        if let Async::Ready(mut srv) = new_srv.new_service().poll().unwrap() {
            let res = srv.call(()).poll();
            assert!(res.is_err());
            assert_eq!(res.err().unwrap(), Error);
        } else {
            panic!()
        }
    }
}
