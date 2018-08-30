use std::marker;

use futures::{Async, Future, Poll};
use {NewService, Service};

/// `Map` service combinator
pub struct Map<A, F, R> {
    a: A,
    f: F,
    r: marker::PhantomData<R>,
}

impl<A, F, R> Map<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R,
{
    /// Create new `Map` combinator
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            r: marker::PhantomData,
        }
    }
}

impl<A, F, R> Service for Map<A, F, R>
where
    A: Service,
    F: Fn(A::Response) -> R,
    F: Clone,
{
    type Request = A::Request;
    type Response = R;
    type Error = A::Error;
    type Future = MapFuture<A, F, R>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.a.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        MapFuture::new(self.a.call(req), self.f.clone())
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
