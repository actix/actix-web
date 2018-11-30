use std::marker::PhantomData;

use futures::{Async, Future, Poll};

use super::{NewService, Service};

/// Service for the `map_err` combinator, changing the type of a service's
/// error.
///
/// This is created by the `ServiceExt::map_err` method.
pub struct MapErr<A, F, E> {
    service: A,
    f: F,
    _t: PhantomData<E>,
}

impl<A, F, E> MapErr<A, F, E> {
    /// Create new `MapErr` combinator
    pub fn new<Request>(service: A, f: F) -> Self
    where
        A: Service<Request>,
        F: Fn(A::Error) -> E,
    {
        Self {
            service,
            f,
            _t: PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapErr<A, F, E>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        MapErr {
            service: self.service.clone(),
            f: self.f.clone(),
            _t: PhantomData,
        }
    }
}

impl<A, F, E, Request> Service<Request> for MapErr<A, F, E>
where
    A: Service<Request>,
    F: Fn(A::Error) -> E + Clone,
{
    type Response = A::Response;
    type Error = E;
    type Future = MapErrFuture<A, F, E, Request>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(&self.f)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        MapErrFuture::new(self.service.call(req), self.f.clone())
    }
}

pub struct MapErrFuture<A, F, E, Request>
where
    A: Service<Request>,
    F: Fn(A::Error) -> E,
{
    f: F,
    fut: A::Future,
}

impl<A, F, E, Request> MapErrFuture<A, F, E, Request>
where
    A: Service<Request>,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrFuture { f, fut }
    }
}

impl<A, F, E, Request> Future for MapErrFuture<A, F, E, Request>
where
    A: Service<Request>,
    F: Fn(A::Error) -> E,
{
    type Item = A::Response;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(&self.f)
    }
}

/// NewService for the `map_err` combinator, changing the type of a new
/// service's error.
///
/// This is created by the `NewServiceExt::map_err` method.
pub struct MapErrNewService<A, F, E> {
    a: A,
    f: F,
    e: PhantomData<E>,
}

impl<A, F, E> MapErrNewService<A, F, E> {
    /// Create new `MapErr` new service instance
    pub fn new<Request>(a: A, f: F) -> Self
    where
        A: NewService<Request>,
        F: Fn(A::Error) -> E,
    {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapErrNewService<A, F, E>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: PhantomData,
        }
    }
}

impl<A, F, E, Request> NewService<Request> for MapErrNewService<A, F, E>
where
    A: NewService<Request>,
    F: Fn(A::Error) -> E + Clone,
{
    type Response = A::Response;
    type Error = E;
    type Service = MapErr<A::Service, F, E>;

    type InitError = A::InitError;
    type Future = MapErrNewServiceFuture<A, F, E, Request>;

    fn new_service(&self) -> Self::Future {
        MapErrNewServiceFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct MapErrNewServiceFuture<A, F, E, Request>
where
    A: NewService<Request>,
    F: Fn(A::Error) -> E,
{
    fut: A::Future,
    f: F,
}

impl<A, F, E, Request> MapErrNewServiceFuture<A, F, E, Request>
where
    A: NewService<Request>,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrNewServiceFuture { f, fut }
    }
}

impl<A, F, E, Request> Future for MapErrNewServiceFuture<A, F, E, Request>
where
    A: NewService<Request>,
    F: Fn(A::Error) -> E + Clone,
{
    type Item = MapErr<A::Service, F, E>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(MapErr::new(service, self.f.clone())))
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

    #[test]
    fn test_poll_ready() {
        let mut srv = Srv.map_err(|_| "error");
        let res = srv.poll_ready();
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "error");
    }

    #[test]
    fn test_call() {
        let mut srv = Srv.map_err(|_| "error");
        let res = srv.call(()).poll();
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "error");
    }

    #[test]
    fn test_new_service() {
        let blank = || Ok::<_, ()>(Srv);
        let new_srv = blank.into_new_service().map_err(|_| "error");
        if let Async::Ready(mut srv) = new_srv.new_service().poll().unwrap() {
            let res = srv.call(()).poll();
            assert!(res.is_err());
            assert_eq!(res.err().unwrap(), "error");
        } else {
            panic!()
        }
    }
}
