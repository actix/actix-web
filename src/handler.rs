use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http::{Error, Response};
use actix_service::{Service, ServiceFactory};
use futures_util::future::{ready, Ready};
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
/// Extract arguments from request, run factory function and make response.
pub struct Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    O: Responder,
{
    hnd: F,
    _t: PhantomData<(T, R, O)>,
}

impl<F, T, R, O> Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
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
    T: FromRequest,
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

impl<F, T, R, O> ServiceFactory for Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    O: Responder,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = Ready<Result<Self::Service, ()>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ready(Ok(self.clone()))
    }
}

// Handler is both it's ServiceFactory and Service Type.
impl<F, T, R, O> Service for Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    O: Responder,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = HandlerServiceFuture<F, T, R, O>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let (req, mut payload) = req.into_parts();
        let fut = T::from_request(&req, &mut payload);
        HandlerServiceFuture::Extract(fut, Some(req), self.hnd.clone())
    }
}

#[doc(hidden)]
#[pin_project(project = HandlerProj)]
pub enum HandlerServiceFuture<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    O: Responder,
{
    Extract(#[pin] T::Future, Option<HttpRequest>, F),
    Handle(#[pin] R, Option<HttpRequest>),
    Respond(#[pin] O::Future, Option<HttpRequest>),
}

impl<F, T, R, O> Future for HandlerServiceFuture<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    O: Responder,
{
    // Error type in this future is a placeholder type.
    // all instances of error must be converted to ServiceResponse and return in Ok.
    type Output = Result<ServiceResponse, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                HandlerProj::Extract(fut, req, handle) => {
                    let item = match ready!(fut.poll(cx)) {
                        Ok(item) => item,
                        Err(e) => {
                            let res: Response = e.into().into();
                            let req = req.take().unwrap();
                            return Poll::Ready(Ok(ServiceResponse::new(req, res)));
                        }
                    };
                    let fut = handle.call(item);
                    let state = HandlerServiceFuture::Handle(fut, req.take());
                    self.as_mut().set(state);
                }
                HandlerProj::Handle(fut, req) => {
                    let res = ready!(fut.poll(cx));
                    let fut = res.respond_to(req.as_ref().unwrap());
                    let state = HandlerServiceFuture::Respond(fut, req.take());
                    self.as_mut().set(state);
                }
                HandlerProj::Respond(fut, req) => {
                    let res = ready!(fut.poll(cx)).unwrap_or_else(|e| e.into().into());
                    let req = req.take().unwrap();
                    return Poll::Ready(Ok(ServiceResponse::new(req, res)));
                }
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
