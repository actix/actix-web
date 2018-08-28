use std::marker;

use futures::{Async, Future, Poll};
use tower_service::{NewService, Service};

/// `MapErr` service combinator
pub struct MapErr<A, F, E> {
    a: A,
    f: F,
    e: marker::PhantomData<E>,
}

impl<A, F, E> MapErr<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
{
    /// Create new `MapErr` combinator
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: marker::PhantomData,
        }
    }
}

impl<A, F, E> Service for MapErr<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
    F: Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;
    type Future = MapErrFuture<A, F, E>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.a.poll_ready().map_err(&self.f)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        MapErrFuture::new(self.a.call(req), self.f.clone())
    }
}

pub struct MapErrFuture<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
{
    f: F,
    fut: A::Future,
}

impl<A, F, E> MapErrFuture<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrFuture { f, fut }
    }
}

impl<A, F, E> Future for MapErrFuture<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
{
    type Item = A::Response;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(&self.f)
    }
}

/// `MapErrNewService` new service combinator
pub struct MapErrNewService<A, F, E> {
    a: A,
    f: F,
    e: marker::PhantomData<E>,
}

impl<A, F, E> MapErrNewService<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E,
{
    /// Create new `MapErr` new service instance
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: marker::PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapErrNewService<A, F, E>
where
    A: NewService + Clone,
    F: Fn(A::Error) -> E + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: marker::PhantomData,
        }
    }
}

impl<A, F, E> NewService for MapErrNewService<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E + Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;
    type Service = MapErr<A::Service, F, E>;

    type InitError = A::InitError;
    type Future = MapErrNewServiceFuture<A, F, E>;

    fn new_service(&self) -> Self::Future {
        MapErrNewServiceFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct MapErrNewServiceFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E,
{
    fut: A::Future,
    f: F,
}

impl<A, F, E> MapErrNewServiceFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrNewServiceFuture { f, fut }
    }
}

impl<A, F, E> Future for MapErrNewServiceFuture<A, F, E>
where
    A: NewService,
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
