use std::marker;

use futures::{Async, Future, Poll};

use super::{NewService, Service};

/// Service for the `map` combinator, changing the type of a service's response.
///
/// This is created by the `ServiceExt::map` method.
pub struct Map<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R,
{
    service: A,
    f: F,
}

impl<A, F, R> Map<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R,
{
    /// Create new `Map` combinator
    pub fn new(service: A, f: F) -> Self {
        Self { service, f }
    }
}

impl<A, F, R> Clone for Map<A, F, R>
where
    A: Service + Clone,
    F: Fn(A::Response) -> R + Clone,
{
    fn clone(&self) -> Self {
        Map {
            service: self.service.clone(),
            f: self.f.clone(),
        }
    }
}

impl<A, F, R> Service for Map<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R + Clone,
{
    type Request = A::Request;
    type Response = R;
    type Error = A::Error;
    type Future = MapFuture<A, F, R>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        MapFuture::new(self.service.call(req), self.f.clone())
    }
}

pub struct MapFuture<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R,
{
    f: F,
    fut: A::Future,
}

impl<A, F, R> MapFuture<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapFuture { f, fut }
    }
}

impl<A, F, R> Future for MapFuture<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R,
{
    type Item = R;
    type Error = A::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(resp) => Ok(Async::Ready((self.f)(resp))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

/// `MapNewService` new service combinator
pub struct MapNewService<A, F, R> {
    a: A,
    f: F,
    r: marker::PhantomData<R>,
}

impl<A, F, R> MapNewService<A, F, R>
where
    A: NewService,
    F: Fn(A::Response) -> R,
{
    /// Create new `Map` new service instance
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            r: marker::PhantomData,
        }
    }
}

impl<A, F, R> Clone for MapNewService<A, F, R>
where
    A: NewService + Clone,
    F: Fn(A::Response) -> R + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            r: marker::PhantomData,
        }
    }
}

impl<A, F, R> NewService for MapNewService<A, F, R>
where
    A: NewService,
    F: Fn(A::Response) -> R + Clone,
{
    type Request = A::Request;
    type Response = R;
    type Error = A::Error;
    type Service = Map<A::Service, F, R>;

    type InitError = A::InitError;
    type Future = MapNewServiceFuture<A, F, R>;

    fn new_service(&self) -> Self::Future {
        MapNewServiceFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct MapNewServiceFuture<A, F, R>
where
    A: NewService,
    F: Fn(A::Response) -> R,
{
    fut: A::Future,
    f: Option<F>,
}

impl<A, F, R> MapNewServiceFuture<A, F, R>
where
    A: NewService,
    F: Fn(A::Response) -> R,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapNewServiceFuture { f: Some(f), fut }
    }
}

impl<A, F, R> Future for MapNewServiceFuture<A, F, R>
where
    A: NewService,
    F: Fn(A::Response) -> R,
{
    type Item = Map<A::Service, F, R>;
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
    use service::{Service, ServiceExt};

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
}
