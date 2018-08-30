use std::marker::PhantomData;

use futures::{Async, Future, Poll};
use {NewService, Service};

/// `Apply` service combinator
pub struct Apply<T, F, R, Req, Resp, Err> {
    service: T,
    f: F,
    r: PhantomData<R>,
    r1: PhantomData<Req>,
    r2: PhantomData<Resp>,
    e: PhantomData<Err>,
}

impl<T, F, R, Req, Resp, Err> Apply<T, F, R, Req, Resp, Err>
where
    T: Service,
    F: Fn(Req, &mut T) -> R,
    R: Future<Item = Resp, Error = Err>,
{
    /// Create new `Apply` combinator
    pub fn new(f: F, service: T) -> Self {
        Self {
            service,
            f,
            r: PhantomData,
            r1: PhantomData,
            r2: PhantomData,
            e: PhantomData,
        }
    }
}

impl<T, F, R, Req, Resp, Err> Service for Apply<T, F, R, Req, Resp, Err>
where
    T: Service,
    T::Error: Into<Err>,
    F: Fn(Req, &mut T) -> R,
    R: Future<Item = Resp, Error = Err>,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err;
    type Future = R;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|e| e.into())
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        (self.f)(req, &mut self.service)
    }
}

/// `ApplyNewService` new service combinator
pub struct ApplyNewService<T, F, R, Req, Resp, Err> {
    service: T,
    f: F,
    r: PhantomData<R>,
    r1: PhantomData<Req>,
    r2: PhantomData<Resp>,
    e: PhantomData<Err>,
}

impl<T, F, R, Req, Resp, Err> ApplyNewService<T, F, R, Req, Resp, Err>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: Future<Item = Resp, Error = Err>,
{
    /// Create new `Partial` new service instance
    pub fn new(f: F, service: T) -> Self {
        Self {
            service,
            f,
            r: PhantomData,
            r1: PhantomData,
            r2: PhantomData,
            e: PhantomData,
        }
    }
}

impl<T, F, R, Req, Resp, Err> Clone for ApplyNewService<T, F, R, Req, Resp, Err>
where
    T: NewService + Clone,
    F: Fn(Req, &mut T::Service) -> R + Clone,
    R: Future<Item = Resp, Error = Err>,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
            r1: PhantomData,
            r2: PhantomData,
            e: PhantomData,
        }
    }
}

impl<T, F, R, Req, Resp, Err> NewService for ApplyNewService<T, F, R, Req, Resp, Err>
where
    T: NewService,
    T::Error: Into<Err>,
    F: Fn(Req, &mut T::Service) -> R + Clone,
    R: Future<Item = Resp, Error = Err>,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err;
    type Service = Apply<T::Service, F, R, Req, Resp, Err>;

    type InitError = T::InitError;
    type Future = ApplyNewServiceFuture<T, F, R, Req, Resp, Err>;

    fn new_service(&self) -> Self::Future {
        ApplyNewServiceFuture::new(self.service.new_service(), self.f.clone())
    }
}

pub struct ApplyNewServiceFuture<T, F, R, Req, Resp, Err>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: Future<Item = Resp, Error = Err>,
{
    fut: T::Future,
    f: Option<F>,
    r: PhantomData<R>,
    r1: PhantomData<Req>,
    r2: PhantomData<Resp>,
    e: PhantomData<Err>,
}

impl<T, F, R, Req, Resp, Err> ApplyNewServiceFuture<T, F, R, Req, Resp, Err>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: Future<Item = Resp, Error = Err>,
{
    fn new(fut: T::Future, f: F) -> Self {
        ApplyNewServiceFuture {
            f: Some(f),
            fut,
            r: PhantomData,
            r1: PhantomData,
            r2: PhantomData,
            e: PhantomData,
        }
    }
}

impl<T, F, R, Req, Resp, Err> Future for ApplyNewServiceFuture<T, F, R, Req, Resp, Err>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: Future<Item = Resp, Error = Err>,
{
    type Item = Apply<T::Service, F, R, Req, Resp, Err>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(Apply::new(self.f.take().unwrap(), service)))
        } else {
            Ok(Async::NotReady)
        }
    }
}
