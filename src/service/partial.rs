use std::marker::PhantomData;

use futures::{Async, Future, Poll};
use {NewService, Service};

/// `Partial` service combinator
pub struct Partial<A, F, Req, Resp> {
    a: A,
    f: F,
    r1: PhantomData<Req>,
    r2: PhantomData<Resp>,
}

impl<A, F, Req, Resp> Partial<A, F, Req, Resp>
where
    A: Service,
    F: Fn(Req) -> (A::Request, Resp),
{
    /// Create new `Partial` combinator
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            r1: PhantomData,
            r2: PhantomData,
        }
    }
}

impl<A, F, Req, Resp> Service for Partial<A, F, Req, Resp>
where
    A: Service,
    F: Fn(Req) -> (A::Request, Resp),
{
    type Request = Req;
    type Response = (A::Response, Resp);
    type Error = A::Error;
    type Future = PartialFuture<A, Resp>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.a.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let (req, resp) = (self.f)(req);
        PartialFuture::new(self.a.call(req), resp)
    }
}

pub struct PartialFuture<A, Resp>
where
    A: Service,
{
    fut: A::Future,
    resp: Option<Resp>,
}

impl<A, Resp> PartialFuture<A, Resp>
where
    A: Service,
{
    fn new(fut: A::Future, resp: Resp) -> Self {
        PartialFuture {
            fut,
            resp: Some(resp),
        }
    }
}

impl<A, Resp> Future for PartialFuture<A, Resp>
where
    A: Service,
{
    type Item = (A::Response, Resp);
    type Error = A::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(resp) => Ok(Async::Ready((resp, self.resp.take().unwrap()))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

/// `PartialNewService` new service combinator
pub struct PartialNewService<A, F, Req, Resp> {
    a: A,
    f: F,
    r1: PhantomData<Req>,
    r2: PhantomData<Resp>,
}

impl<A, F, Req, Resp> PartialNewService<A, F, Req, Resp>
where
    A: NewService,
    F: Fn(Req) -> (A::Request, Resp),
{
    /// Create new `Partial` new service instance
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            r1: PhantomData,
            r2: PhantomData,
        }
    }
}

impl<A, F, Req, Resp> Clone for PartialNewService<A, F, Req, Resp>
where
    A: NewService + Clone,
    F: Fn(Req) -> (A::Request, Resp) + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            r1: PhantomData,
            r2: PhantomData,
        }
    }
}

impl<A, F, Req, Resp> NewService for PartialNewService<A, F, Req, Resp>
where
    A: NewService,
    F: Fn(Req) -> (A::Request, Resp) + Clone,
{
    type Request = Req;
    type Response = (A::Response, Resp);
    type Error = A::Error;
    type Service = Partial<A::Service, F, Req, Resp>;

    type InitError = A::InitError;
    type Future = PartialNewServiceFuture<A, F, Req, Resp>;

    fn new_service(&self) -> Self::Future {
        PartialNewServiceFuture::new(self.a.new_service(), self.f.clone())
    }
}

pub struct PartialNewServiceFuture<A, F, Req, Resp>
where
    A: NewService,
    F: Fn(Req) -> (A::Request, Resp),
{
    fut: A::Future,
    f: Option<F>,
    r1: PhantomData<Req>,
    r2: PhantomData<Resp>,
}

impl<A, F, Req, Resp> PartialNewServiceFuture<A, F, Req, Resp>
where
    A: NewService,
    F: Fn(Req) -> (A::Request, Resp),
{
    fn new(fut: A::Future, f: F) -> Self {
        PartialNewServiceFuture {
            f: Some(f),
            fut,
            r1: PhantomData,
            r2: PhantomData,
        }
    }
}

impl<A, F, Req, Resp> Future for PartialNewServiceFuture<A, F, Req, Resp>
where
    A: NewService,
    F: Fn(Req) -> (A::Request, Resp),
{
    type Item = Partial<A::Service, F, Req, Resp>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(Partial::new(service, self.f.take().unwrap())))
        } else {
            Ok(Async::NotReady)
        }
    }
}
