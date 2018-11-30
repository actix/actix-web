use std::marker::PhantomData;

use futures::{Async, Future, Poll};

use super::{NewService, Service};

/// Service for the `map` combinator, changing the type of a service's response.
///
/// This is created by the `ServiceExt::map` method.
pub struct Map<A, F, Response> {
    service: A,
    f: F,
    _t: PhantomData<Response>,
}

impl<A, F, Response> Map<A, F, Response> {
    /// Create new `Map` combinator
    pub fn new<Request>(service: A, f: F) -> Self
    where
        A: Service<Request>,
        F: Fn(A::Response) -> Response,
    {
        Self {
            service,
            f,
            _t: PhantomData,
        }
    }
}

impl<A, F, Response> Clone for Map<A, F, Response>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Map {
            service: self.service.clone(),
            f: self.f.clone(),
            _t: PhantomData,
        }
    }
}

impl<A, F, Request, Response> Service<Request> for Map<A, F, Response>
where
    A: Service<Request>,
    F: Fn(A::Response) -> Response + Clone,
{
    type Response = Response;
    type Error = A::Error;
    type Future = MapFuture<A, F, Request, Response>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: Request) -> Self::Future {
        MapFuture::new(self.service.call(req), self.f.clone())
    }
}

pub struct MapFuture<A, F, Request, Response>
where
    A: Service<Request>,
    F: Fn(A::Response) -> Response,
{
    f: F,
    fut: A::Future,
}

impl<A, F, Request, Response> MapFuture<A, F, Request, Response>
where
    A: Service<Request>,
    F: Fn(A::Response) -> Response,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapFuture { f, fut }
    }
}

impl<A, F, Request, Response> Future for MapFuture<A, F, Request, Response>
where
    A: Service<Request>,
    F: Fn(A::Response) -> Response,
{
    type Item = Response;
    type Error = A::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(resp) => Ok(Async::Ready((self.f)(resp))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

/// `MapNewService` new service combinator
pub struct MapNewService<A, F, Response> {
    a: A,
    f: F,
    r: PhantomData<Response>,
}

impl<A, F, Response> MapNewService<A, F, Response> {
    /// Create new `Map` new service instance
    pub fn new<Request>(a: A, f: F) -> Self
    where
        A: NewService<Request>,
        F: Fn(A::Response) -> Response,
    {
        Self {
            a,
            f,
            r: PhantomData,
        }
    }
}

impl<A, F, Response> Clone for MapNewService<A, F, Response>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<A, F, Request, Response> NewService<Request> for MapNewService<A, F, Response>
where
    A: NewService<Request>,
    F: Fn(A::Response) -> Response + Clone,
{
    type Response = Response;
    type Error = A::Error;
    type Service = Map<A::Service, F, Response>;

    type InitError = A::InitError;
    type Future = MapNewServiceFuture<A, F, Request, Response>;

    fn new_service(&self) -> Self::Future {
        MapNewServiceFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct MapNewServiceFuture<A, F, Request, Response>
where
    A: NewService<Request>,
    F: Fn(A::Response) -> Response,
{
    fut: A::Future,
    f: Option<F>,
}

impl<A, F, Request, Response> MapNewServiceFuture<A, F, Request, Response>
where
    A: NewService<Request>,
    F: Fn(A::Response) -> Response,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapNewServiceFuture { f: Some(f), fut }
    }
}

impl<A, F, Request, Response> Future for MapNewServiceFuture<A, F, Request, Response>
where
    A: NewService<Request>,
    F: Fn(A::Response) -> Response,
{
    type Item = Map<A::Service, F, Response>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(Map::new(service, self.f.take().unwrap())))
        } else {
            Ok(Async::NotReady)
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{ok, FutureResult};

    use super::*;
    use service::{IntoNewService, NewServiceExt, Service, ServiceExt};

    struct Srv;
    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = FutureResult<(), ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Ok(Async::Ready(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[test]
    fn test_poll_ready() {
        let mut srv = Srv.map(|_| "ok");
        let res = srv.poll_ready();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(()));
    }

    #[test]
    fn test_call() {
        let mut srv = Srv.map(|_| "ok");
        let res = srv.call(()).poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready("ok"));
    }

    #[test]
    fn test_new_service() {
        let blank = || Ok::<_, ()>(Srv);
        let new_srv = blank.into_new_service().map(|_| "ok");
        if let Async::Ready(mut srv) = new_srv.new_service().poll().unwrap() {
            let res = srv.call(()).poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Async::Ready("ok"));
        } else {
            panic!()
        }
    }
}
