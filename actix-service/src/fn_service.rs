use std::marker;

use futures::{
    future::{ok, FutureResult},
    Async, IntoFuture, Poll,
};

use super::{IntoNewService, IntoService, NewService, Service};

pub struct FnService<F, Req, Resp, E, Fut>
where
    F: FnMut(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    f: F,
    _t: marker::PhantomData<(Req, Resp, E)>,
}

impl<F, Req, Resp, E, Fut> FnService<F, Req, Resp, E, Fut>
where
    F: FnMut(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    pub fn new(f: F) -> Self {
        FnService {
            f,
            _t: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, E, Fut> Clone for FnService<F, Req, Resp, E, Fut>
where
    F: FnMut(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
    fn clone(&self) -> Self {
        FnService {
            f: self.f.clone(),
            _t: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, E, Fut> Service<Req> for FnService<F, Req, Resp, E, Fut>
where
    F: FnMut(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = E>,
{
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

impl<F, Req, Resp, Err, Fut> IntoService<FnService<F, Req, Resp, Err, Fut>, Req> for F
where
    F: FnMut(Req) -> Fut + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn into_service(self) -> FnService<F, Req, Resp, Err, Fut> {
        FnService::new(self)
    }
}

pub struct FnNewService<F, Req, Resp, Err, Fut>
where
    F: FnMut(Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    f: F,
    _t: marker::PhantomData<(Req, Resp, Err)>,
}

impl<F, Req, Resp, Err, Fut> FnNewService<F, Req, Resp, Err, Fut>
where
    F: FnMut(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    pub fn new(f: F) -> Self {
        FnNewService {
            f,
            _t: marker::PhantomData,
        }
    }
}

impl<F, Req, Resp, Err, Fut> NewService<Req> for FnNewService<F, Req, Resp, Err, Fut>
where
    F: FnMut(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    type Response = Resp;
    type Error = Err;
    type Service = FnService<F, Req, Resp, Err, Fut>;
    type InitError = ();
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(FnService::new(self.f.clone()))
    }
}

impl<F, Req, Resp, Err, Fut> IntoNewService<FnNewService<F, Req, Resp, Err, Fut>, Req> for F
where
    F: FnMut(Req) -> Fut + Clone + 'static,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn into_new_service(self) -> FnNewService<F, Req, Resp, Err, Fut> {
        FnNewService::new(self)
    }
}

impl<F, Req, Resp, Err, Fut> Clone for FnNewService<F, Req, Resp, Err, Fut>
where
    F: FnMut(Req) -> Fut + Clone,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}
