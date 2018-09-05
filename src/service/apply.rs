use std::marker::PhantomData;

use futures::{Async, Future, IntoFuture, Poll};
use {IntoNewService, NewService, Service};

/// `Apply` service combinator
pub struct Apply<T, F, R, Req> {
    service: T,
    f: F,
    r: PhantomData<Fn(Req) -> R>,
}

impl<T, F, R, Req> Apply<T, F, R, Req>
where
    T: Service,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T) -> R,
    R: IntoFuture,
{
    /// Create new `Apply` combinator
    pub fn new(f: F, service: T) -> Self {
        Self {
            service,
            f,
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Clone for Apply<T, F, R, Req>
where
    T: Service + Clone,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T) -> R + Clone,
    R: IntoFuture,
{
    fn clone(&self) -> Self {
        Apply {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Service for Apply<T, F, R, Req>
where
    T: Service,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T) -> R,
    R: IntoFuture,
{
    type Request = Req;
    type Response = <R::Future as Future>::Item;
    type Error = <R::Future as Future>::Error;
    type Future = R::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|e| e.into())
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        (self.f)(req, &mut self.service).into_future()
    }
}

/// `ApplyNewService` new service combinator
pub struct ApplyNewService<T, F, R, Req> {
    service: T,
    f: F,
    r: PhantomData<Fn(Req) -> R>,
}

impl<T, F, R, Req> ApplyNewService<T, F, R, Req>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    /// Create new `ApplyNewService` new service instance
    pub fn new<F1: IntoNewService<T>>(f: F, service: F1) -> Self {
        Self {
            f,
            service: service.into_new_service(),
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Clone for ApplyNewService<T, F, R, Req>
where
    T: NewService + Clone,
    F: Fn(Req, &mut T::Service) -> R + Clone,
    R: IntoFuture,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> NewService for ApplyNewService<T, F, R, Req>
where
    T: NewService,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T::Service) -> R + Clone,
    R: IntoFuture,
{
    type Request = Req;
    type Response = <R::Future as Future>::Item;
    type Error = <R::Future as Future>::Error;
    type Service = Apply<T::Service, F, R, Req>;

    type InitError = T::InitError;
    type Future = ApplyNewServiceFuture<T, F, R, Req>;

    fn new_service(&self) -> Self::Future {
        ApplyNewServiceFuture::new(self.service.new_service(), self.f.clone())
    }
}

pub struct ApplyNewServiceFuture<T, F, R, Req>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    fut: T::Future,
    f: Option<F>,
    r: PhantomData<Fn(Req) -> R>,
}

impl<T, F, R, Req> ApplyNewServiceFuture<T, F, R, Req>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    fn new(fut: T::Future, f: F) -> Self {
        ApplyNewServiceFuture {
            f: Some(f),
            fut,
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Future for ApplyNewServiceFuture<T, F, R, Req>
where
    T: NewService,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    type Item = Apply<T::Service, F, R, Req>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(Apply::new(self.f.take().unwrap(), service)))
        } else {
            Ok(Async::NotReady)
        }
    }
}
