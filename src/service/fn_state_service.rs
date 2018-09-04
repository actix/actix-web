use std::marker;

use futures::{Async, Future, IntoFuture, Poll};
use tower_service::{NewService, Service};

pub struct FnStateService<S, F, Req, Resp, Err, Fut>
where
    F: Fn(&mut S, Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    f: F,
    state: S,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err: marker::PhantomData<Err>,
}

impl<S, F, Req, Resp, Err, Fut> FnStateService<S, F, Req, Resp, Err, Fut>
where
    F: Fn(&mut S, Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    pub fn new(state: S, f: F) -> Self {
        FnStateService {
            f,
            state,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err: marker::PhantomData,
        }
    }
}

impl<S, F, Req, Resp, Err, Fut> Service for FnStateService<S, F, Req, Resp, Err, Fut>
where
    F: Fn(&mut S, Req) -> Fut,
    Fut: IntoFuture<Item = Resp, Error = Err>,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err;
    type Future = Fut::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Req) -> Self::Future {
        (self.f)(&mut self.state, req).into_future()
    }
}

/// `NewService` for state and handler functions
pub struct FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> {
    f: F1,
    state: F2,
    s: marker::PhantomData<S>,
    req: marker::PhantomData<Req>,
    resp: marker::PhantomData<Resp>,
    err1: marker::PhantomData<Err1>,
    err2: marker::PhantomData<Err2>,
    fut1: marker::PhantomData<Fut1>,
    fut2: marker::PhantomData<Fut2>,
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
    FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
{
    fn new(f: F1, state: F2) -> Self {
        FnStateNewService {
            f,
            state,
            s: marker::PhantomData,
            req: marker::PhantomData,
            resp: marker::PhantomData,
            err1: marker::PhantomData,
            err2: marker::PhantomData,
            fut1: marker::PhantomData,
            fut2: marker::PhantomData,
        }
    }
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> NewService
    for FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
where
    S: 'static,
    F1: Fn(&mut S, Req) -> Fut1 + Clone + 'static,
    F2: Fn() -> Fut2,
    Fut1: IntoFuture<Item = Resp, Error = Err1> + 'static,
    Fut2: IntoFuture<Item = S, Error = Err2> + 'static,
    Req: 'static,
    Resp: 'static,
    Err1: 'static,
    Err2: 'static,
{
    type Request = Req;
    type Response = Resp;
    type Error = Err1;
    type Service = FnStateService<S, F1, Req, Resp, Err1, Fut1>;
    type InitError = Err2;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self) -> Self::Future {
        let f = self.f.clone();
        Box::new(
            (self.state)()
                .into_future()
                .and_then(move |state| Ok(FnStateService::new(state, f))),
        )
    }
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> From<(F1, F2)>
    for FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
where
    S: 'static,
    F1: Fn(&mut S, Req) -> Fut1 + Clone + 'static,
    F2: Fn() -> Fut2,
    Fut1: IntoFuture<Item = Resp, Error = Err1> + 'static,
    Fut2: IntoFuture<Item = S, Error = Err2> + 'static,
    Req: 'static,
    Resp: 'static,
    Err1: 'static,
    Err2: 'static,
{
    fn from(data: (F1, F2)) -> FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> {
        FnStateNewService::new(data.0, data.1)
    }
}

impl<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2> Clone
    for FnStateNewService<S, F1, F2, Req, Resp, Err1, Err2, Fut1, Fut2>
where
    F1: Fn(&mut S, Req) -> Fut1 + Clone + 'static,
    F2: Fn() -> Fut2 + Clone,
    Fut1: IntoFuture<Item = Resp, Error = Err1>,
    Fut2: IntoFuture<Item = S, Error = Err2>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone(), self.state.clone())
    }
}
