use std::marker;

use futures::{Async, Future, Poll};
use {NewService, Service};

/// `MapReq` service combinator
pub struct MapReq<A, F, R> {
    a: A,
    f: F,
    r: marker::PhantomData<R>,
}

impl<A, F, R> MapReq<A, F, R>
where
    A: Service,
    F: Fn(R) -> A::Request,
{
    /// Create new `MapReq` combinator
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            r: marker::PhantomData,
        }
    }
}

impl<A, F, R> Service for MapReq<A, F, R>
where
    A: Service,
    F: Fn(R) -> A::Request,
    F: Clone,
{
    type Request = R;
    type Response = A::Response;
    type Error = A::Error;
    type Future = A::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.a.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.a.call((self.f)(req))
    }
}

/// `MapReqNewService` new service combinator
pub struct MapReqNewService<A, F, R> {
    a: A,
    f: F,
    r: marker::PhantomData<R>,
}

impl<A, F, R> MapReqNewService<A, F, R>
where
    A: NewService,
    F: Fn(R) -> A::Request,
{
    /// Create new `MapReq` new service instance
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            r: marker::PhantomData,
        }
    }
}

impl<A, F, R> Clone for MapReqNewService<A, F, R>
where
    A: NewService + Clone,
    F: Fn(R) -> A::Request + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            r: marker::PhantomData,
        }
    }
}

impl<A, F, R> NewService for MapReqNewService<A, F, R>
where
    A: NewService,
    F: Fn(R) -> A::Request + Clone,
{
    type Request = R;
    type Response = A::Response;
    type Error = A::Error;
    type Service = MapReq<A::Service, F, R>;

    type InitError = A::InitError;
    type Future = MapReqNewServiceFuture<A, F, R>;

    fn new_service(&self) -> Self::Future {
        MapReqNewServiceFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct MapReqNewServiceFuture<A, F, R>
where
    A: NewService,
    F: Fn(R) -> A::Request,
{
    fut: A::Future,
    f: Option<F>,
    r: marker::PhantomData<R>,
}

impl<A, F, R> MapReqNewServiceFuture<A, F, R>
where
    A: NewService,
    F: Fn(R) -> A::Request,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapReqNewServiceFuture {
            f: Some(f),
            fut,
            r: marker::PhantomData,
        }
    }
}

impl<A, F, R> Future for MapReqNewServiceFuture<A, F, R>
where
    A: NewService,
    F: Fn(R) -> A::Request,
{
    type Item = MapReq<A::Service, F, R>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(MapReq::new(service, self.f.take().unwrap())))
        } else {
            Ok(Async::NotReady)
        }
    }
}
