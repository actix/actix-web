use std::marker;

use futures::{
    future::{ok, FutureResult},
    Async, IntoFuture, Poll,
};
use tower_service::{NewService, Service};

pub struct FnService<F, Req, Resp, E, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    f: F,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err: marker::PhantomData<E>,
}

impl<F, Req, Resp, E, Fut> FnService<F, Req, Resp, E, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    pub fn new(f: F) -> Self {
        FnService {
            f,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, E, Fut> Clone for FnService<F, Req, Resp, E, Fut>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    fn clone(&self) -> Self {
        FnService {
            f: self.f.clone(),
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, E, Fut> Service for FnService<F, Req, Resp, E, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    type Request = Req;
    type Response = Resp;
    type Error = E;
    type Future = Fut::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Req) -> Self::Future {
        (self.f)(req).into_future()
    }
}

pub struct FnNewService<F, Req, Resp, Err, IErr, Fut>
where
    F: Fn(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    f: F,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err: marker::PhantomData<Err>,
    ierr: marker::PhantomData<IErr>,
}

impl<F, Req, Resp, Err, IErr, Fut> FnNewService<F, Req, Resp, Err, IErr, Fut>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    pub fn new(f: F) -> Self {
        FnNewService {
            f,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
            ierr: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, Err, IErr, Fut> NewService for FnNewService<F, Req, Resp, Err, IErr, Fut>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err;
    type Service = FnService<F, Req, Resp, Err, Fut>;
    type InitError = IErr;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(FnService::new(self.f.clone()))
    }
}

impl<F, Req, Resp, Err, IErr, Fut> From<F> for FnNewService<F, Req, Resp, Err, IErr, Fut>
where
    F: Fn(Req) -> Fut + Clone + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn from(f: F) -> FnNewService<F, Req, Resp, Err, IErr, Fut> {
        FnNewService::new(f)
    }
}

impl<F, Req, Resp, Err, IErr, Fut> Clone for FnNewService<F, Req, Resp, Err, IErr, Fut>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}
