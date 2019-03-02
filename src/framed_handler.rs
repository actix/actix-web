use std::marker::PhantomData;
use std::rc::Rc;

use actix_codec::Framed;
use actix_http::{h1::Codec, Error};
use actix_service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};
use log::error;

use crate::handler::FromRequest;
use crate::request::Request;

pub struct FramedError<Io> {
    pub err: Error,
    pub framed: Framed<Io, Codec>,
}

pub struct FramedRequest<S, Io, Ex = ()> {
    req: Request<S>,
    framed: Framed<Io, Codec>,
    param: Ex,
}

impl<S, Io> FramedRequest<S, Io, ()> {
    pub fn new(req: Request<S>, framed: Framed<Io, Codec>) -> Self {
        Self {
            req,
            framed,
            param: (),
        }
    }
}

impl<S, Io, Ex> FramedRequest<S, Io, Ex> {
    pub fn request(&self) -> &Request<S> {
        &self.req
    }

    pub fn request_mut(&mut self) -> &mut Request<S> {
        &mut self.req
    }

    pub fn into_parts(self) -> (Request<S>, Framed<Io, Codec>, Ex) {
        (self.req, self.framed, self.param)
    }

    pub fn map<Ex2, F>(self, op: F) -> FramedRequest<S, Io, Ex2>
    where
        F: FnOnce(Ex) -> Ex2,
    {
        FramedRequest {
            req: self.req,
            framed: self.framed,
            param: op(self.param),
        }
    }
}

/// T handler converter factory
pub trait FramedFactory<S, Io, Ex, T, R, E>: Clone + 'static
where
    R: IntoFuture<Item = (), Error = E>,
    E: Into<Error>,
{
    fn call(&self, framed: Framed<Io, Codec>, param: T, extra: Ex) -> R;
}

#[doc(hidden)]
pub struct FramedHandle<F, S, Io, Ex, T, R, E>
where
    F: FramedFactory<S, Io, Ex, T, R, E>,
    R: IntoFuture<Item = (), Error = E>,
    E: Into<Error>,
{
    hnd: F,
    _t: PhantomData<(S, Io, Ex, T, R, E)>,
}

impl<F, S, Io, Ex, T, R, E> FramedHandle<F, S, Io, Ex, T, R, E>
where
    F: FramedFactory<S, Io, Ex, T, R, E>,
    R: IntoFuture<Item = (), Error = E>,
    E: Into<Error>,
{
    pub fn new(hnd: F) -> Self {
        FramedHandle {
            hnd,
            _t: PhantomData,
        }
    }
}
impl<F, S, Io, Ex, T, R, E> NewService for FramedHandle<F, S, Io, Ex, T, R, E>
where
    F: FramedFactory<S, Io, Ex, T, R, E>,
    R: IntoFuture<Item = (), Error = E>,
    E: Into<Error>,
{
    type Request = (T, FramedRequest<S, Io, Ex>);
    type Response = ();
    type Error = FramedError<Io>;
    type InitError = ();
    type Service = FramedHandleService<F, S, Io, Ex, T, R, E>;
    type Future = FutureResult<Self::Service, ()>;

    fn new_service(&self) -> Self::Future {
        ok(FramedHandleService {
            hnd: self.hnd.clone(),
            _t: PhantomData,
        })
    }
}

#[doc(hidden)]
pub struct FramedHandleService<F, S, Io, Ex, T, R, E>
where
    F: FramedFactory<S, Io, Ex, T, R, E>,
    R: IntoFuture<Item = (), Error = E>,
    E: Into<Error>,
{
    hnd: F,
    _t: PhantomData<(S, Io, Ex, T, R, E)>,
}

impl<F, S, Io, Ex, T, R, E> Service for FramedHandleService<F, S, Io, Ex, T, R, E>
where
    F: FramedFactory<S, Io, Ex, T, R, E>,
    R: IntoFuture<Item = (), Error = E>,
    E: Into<Error>,
{
    type Request = (T, FramedRequest<S, Io, Ex>);
    type Response = ();
    type Error = FramedError<Io>;
    type Future = FramedHandleServiceResponse<Io, R::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (param, framed): (T, FramedRequest<S, Io, Ex>)) -> Self::Future {
        let (_, framed, ex) = framed.into_parts();
        FramedHandleServiceResponse {
            fut: self.hnd.call(framed, param, ex).into_future(),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct FramedHandleServiceResponse<Io, F> {
    fut: F,
    _t: PhantomData<Io>,
}

impl<Io, F> Future for FramedHandleServiceResponse<Io, F>
where
    F: Future<Item = ()>,
    F::Error: Into<Error>,
{
    type Item = ();
    type Error = FramedError<Io>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(res)) => Ok(Async::Ready(res.into())),
            Err(e) => {
                let e: Error = e.into();
                error!("Error in handler: {:?}", e);
                Ok(Async::Ready(()))
            }
        }
    }
}

pub struct FramedExtract<S, Io, Ex, T>
where
    T: FromRequest<S>,
{
    cfg: Rc<T::Config>,
    _t: PhantomData<(Io, Ex)>,
}

impl<S, Io, Ex, T> FramedExtract<S, Io, Ex, T>
where
    T: FromRequest<S> + 'static,
{
    pub fn new(cfg: T::Config) -> FramedExtract<S, Io, Ex, T> {
        FramedExtract {
            cfg: Rc::new(cfg),
            _t: PhantomData,
        }
    }
}
impl<S, Io, Ex, T> NewService for FramedExtract<S, Io, Ex, T>
where
    T: FromRequest<S> + 'static,
{
    type Request = FramedRequest<S, Io, Ex>;
    type Response = (T, FramedRequest<S, Io, Ex>);
    type Error = FramedError<Io>;
    type InitError = ();
    type Service = FramedExtractService<S, Io, Ex, T>;
    type Future = FutureResult<Self::Service, ()>;

    fn new_service(&self) -> Self::Future {
        ok(FramedExtractService {
            cfg: self.cfg.clone(),
            _t: PhantomData,
        })
    }
}

pub struct FramedExtractService<S, Io, Ex, T>
where
    T: FromRequest<S>,
{
    cfg: Rc<T::Config>,
    _t: PhantomData<(Io, Ex)>,
}

impl<S, Io, Ex, T> Service for FramedExtractService<S, Io, Ex, T>
where
    T: FromRequest<S> + 'static,
{
    type Request = FramedRequest<S, Io, Ex>;
    type Response = (T, FramedRequest<S, Io, Ex>);
    type Error = FramedError<Io>;
    type Future = FramedExtractResponse<S, Io, Ex, T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: FramedRequest<S, Io, Ex>) -> Self::Future {
        FramedExtractResponse {
            fut: T::from_request(&req.request(), self.cfg.as_ref()),
            req: Some(req),
        }
    }
}

pub struct FramedExtractResponse<S, Io, Ex, T>
where
    T: FromRequest<S> + 'static,
{
    req: Option<FramedRequest<S, Io, Ex>>,
    fut: T::Future,
}

impl<S, Io, Ex, T> Future for FramedExtractResponse<S, Io, Ex, T>
where
    T: FromRequest<S> + 'static,
{
    type Item = (T, FramedRequest<S, Io, Ex>);
    type Error = FramedError<Io>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(item)) => Ok(Async::Ready((item, self.req.take().unwrap()))),
            Err(err) => Err(FramedError {
                err: err.into(),
                framed: self.req.take().unwrap().into_parts().1,
            }),
        }
    }
}

macro_rules! factory_tuple ({ ($(($nex:tt, $Ex:ident)),+), $(($n:tt, $T:ident)),+} => {
    impl<Func, S, Io, $($Ex,)+ $($T,)+ Res, Err> FramedFactory<S, Io, ($($Ex,)+), ($($T,)+), Res, Err> for Func
    where Func: Fn(Framed<Io, Codec>, $($Ex,)+ $($T,)+) -> Res + Clone + 'static,
         $($T: FromRequest<S> + 'static,)+
          Res: IntoFuture<Item=(), Error=Err> + 'static,
          Err: Into<Error>,
    {
        fn call(&self, framed: Framed<Io, Codec>, param: ($($T,)+), extra: ($($Ex,)+)) -> Res {
            (self)(framed, $(extra.$nex,)+ $(param.$n,)+)
        }
    }
});

macro_rules! factory_tuple_unit ({$(($n:tt, $T:ident)),+} => {
    impl<Func, S, Io, $($T,)+ Res, Err> FramedFactory<S, Io, (), ($($T,)+), Res, Err> for Func
    where Func: Fn(Framed<Io, Codec>, $($T,)+) -> Res + Clone + 'static,
         $($T: FromRequest<S> + 'static,)+
          Res: IntoFuture<Item=(), Error=Err> + 'static,
          Err: Into<Error>,
    {
        fn call(&self, framed: Framed<Io, Codec>, param: ($($T,)+), _extra: () ) -> Res {
            (self)(framed, $(param.$n,)+)
        }
    }
});

#[cfg_attr(rustfmt, rustfmt_skip)]
mod m {
    use super::*;

factory_tuple_unit!((0, A));
factory_tuple!(((0, Aex)), (0, A));
factory_tuple!(((0, Aex), (1, Bex)), (0, A));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A));

factory_tuple_unit!((0, A), (1, B));
factory_tuple!(((0, Aex)), (0, A), (1, B));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B));

factory_tuple_unit!((0, A), (1, B), (2, C));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C));

factory_tuple_unit!((0, A), (1, B), (2, C), (3, D));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C), (3, D));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C), (3, D));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C), (3, D));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C), (3, D));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C), (3, D));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C), (3, D));

factory_tuple_unit!((0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C), (3, D), (4, E));

factory_tuple_unit!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));

factory_tuple_unit!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));

factory_tuple_unit!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));

factory_tuple_unit!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));

factory_tuple_unit!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
factory_tuple!(((0, Aex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
factory_tuple!(((0, Aex), (1, Bex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
factory_tuple!(((0, Aex), (1, Bex), (2, Cex), (3, Dex), (4, Eex), (5, Fex)), (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
}
