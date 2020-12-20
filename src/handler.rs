use std::convert::Infallible;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http::{Error, Response};
use actix_service::{Service, ServiceFactory};
use futures_util::future::{ok, Ready};
use futures_util::ready;
use pin_project::pin_project;

use crate::extract::FromRequest;
use crate::request::HttpRequest;
use crate::responder::Responder;
use crate::service::{ServiceRequest, ServiceResponse};

/// Async handler converter factory
pub trait Factory<T, R, O>: Clone + 'static
where
    R: Future<Output = O>,
    O: Responder,
{
    fn call(&self, param: T) -> R;
}

impl<F, R, O> Factory<(), R, O> for F
where
    F: Fn() -> R + Clone + 'static,
    R: Future<Output = O>,
    O: Responder,
{
    fn call(&self, _: ()) -> R {
        (self)()
    }
}

#[doc(hidden)]
pub struct Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    R: Future<Output = O>,
    O: Responder,
{
    hnd: F,
    _t: PhantomData<(T, R, O)>,
}

impl<F, T, R, O> Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    R: Future<Output = O>,
    O: Responder,
{
    pub fn new(hnd: F) -> Self {
        Handler {
            hnd,
            _t: PhantomData,
        }
    }
}

impl<F, T, R, O> Clone for Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    R: Future<Output = O>,
    O: Responder,
{
    fn clone(&self) -> Self {
        Handler {
            hnd: self.hnd.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, T, R, O> Service for Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    R: Future<Output = O>,
    O: Responder,
{
    type Request = (T, HttpRequest);
    type Response = ServiceResponse;
    type Error = Infallible;
    type Future = HandlerServiceResponse<R, O>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (param, req): (T, HttpRequest)) -> Self::Future {
        let fut = self.hnd.call(param);
        HandlerServiceResponse::Future(fut, Some(req))
    }
}

#[doc(hidden)]
#[pin_project(project = HandlerProj)]
pub enum HandlerServiceResponse<T, R>
where
    T: Future<Output = R>,
    R: Responder,
{
    Future(#[pin] T, Option<HttpRequest>),
    Responder(#[pin] R::Future, Option<HttpRequest>),
}

impl<T, R> Future for HandlerServiceResponse<T, R>
where
    T: Future<Output = R>,
    R: Responder,
{
    type Output = Result<ServiceResponse, Infallible>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                HandlerProj::Future(fut, req) => {
                    let res = ready!(fut.poll(cx));
                    let fut = res.respond_to(req.as_ref().unwrap());
                    let state = HandlerServiceResponse::Responder(fut, req.take());
                    self.as_mut().set(state);
                }
                HandlerProj::Responder(fut, req) => {
                    let res = ready!(fut.poll(cx));
                    let req = req.take().unwrap();
                    return match res {
                        Ok(res) => Poll::Ready(Ok(ServiceResponse::new(req, res))),
                        Err(e) => {
                            let res: Response = e.into().into();
                            Poll::Ready(Ok(ServiceResponse::new(req, res)))
                        }
                    };
                }
            }
        }
    }
}

/// Extract arguments from request
pub struct Extract<T: FromRequest, S> {
    service: S,
    _t: PhantomData<T>,
}

impl<T: FromRequest, S> Extract<T, S> {
    pub fn new(service: S) -> Self {
        Extract {
            service,
            _t: PhantomData,
        }
    }
}

impl<T: FromRequest, S> ServiceFactory for Extract<T, S>
where
    S: Service<
        Request = (T, HttpRequest),
        Response = ServiceResponse,
        Error = Infallible,
    > + Clone,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = ExtractService<T, S>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, ()>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(ExtractService {
            _t: PhantomData,
            service: self.service.clone(),
        })
    }
}

pub struct ExtractService<T: FromRequest, S> {
    service: S,
    _t: PhantomData<T>,
}

impl<T: FromRequest, S> Service for ExtractService<T, S>
where
    S: Service<
        Request = (T, HttpRequest),
        Response = ServiceResponse,
        Error = Infallible,
    > + Clone,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = ExtractResponse<T, S>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let (req, mut payload) = req.into_parts();
        let fut = T::from_request(&req, &mut payload);
        ExtractResponse::Future(fut, Some(req), self.service.clone())
    }
}

#[pin_project(project = ExtractProj)]
pub enum ExtractResponse<T: FromRequest, S: Service> {
    Future(#[pin] T::Future, Option<HttpRequest>, S),
    Response(#[pin] S::Future),
}

impl<T: FromRequest, S> Future for ExtractResponse<T, S>
where
    S: Service<
        Request = (T, HttpRequest),
        Response = ServiceResponse,
        Error = Infallible,
    >,
{
    type Output = Result<ServiceResponse, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                ExtractProj::Future(fut, req, srv) => {
                    let res = ready!(fut.poll(cx));
                    let req = req.take().unwrap();
                    match res {
                        Err(e) => {
                            let req = ServiceRequest::new(req);
                            return Poll::Ready(Ok(req.error_response(e.into())));
                        }
                        Ok(item) => {
                            let fut = srv.call((item, req));
                            self.as_mut().set(ExtractResponse::Response(fut));
                        }
                    }
                }
                ExtractProj::Response(fut) => return fut.poll(cx).map_err(|_| panic!()),
            }
        }
    }
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $(($n:tt, $T:ident)),+} => {
    impl<Func, $($T,)+ Res, O> Factory<($($T,)+), Res, O> for Func
    where Func: Fn($($T,)+) -> Res + Clone + 'static,
          Res: Future<Output = O>,
          O: Responder,
    {
        fn call(&self, param: ($($T,)+)) -> Res {
            (self)($(param.$n,)+)
        }
    }
});

#[rustfmt::skip]
mod m {
    use super::*;

factory_tuple!((0, A));
factory_tuple!((0, A), (1, B));
factory_tuple!((0, A), (1, B), (2, C));
factory_tuple!((0, A), (1, B), (2, C), (3, D));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
}
