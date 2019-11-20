use std::convert::Infallible;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http::{Error, Response};
use actix_service::{Service, ServiceFactory};
use futures::future::{ok, Ready};
use futures::ready;
use pin_project::pin_project;

use crate::extract::FromRequest;
use crate::request::HttpRequest;
use crate::responder::Responder;
use crate::service::{ServiceRequest, ServiceResponse};

/// Handler converter factory
pub trait Factory<T, R>: Clone
where
    R: Responder,
{
    fn call(&self, param: T) -> R;
}

impl<F, R> Factory<(), R> for F
where
    F: Fn() -> R + Clone,
    R: Responder,
{
    fn call(&self, _: ()) -> R {
        (self)()
    }
}

#[doc(hidden)]
pub struct Handler<F, T, R>
where
    F: Factory<T, R>,
    R: Responder,
{
    hnd: F,
    _t: PhantomData<(T, R)>,
}

impl<F, T, R> Handler<F, T, R>
where
    F: Factory<T, R>,
    R: Responder,
{
    pub fn new(hnd: F) -> Self {
        Handler {
            hnd,
            _t: PhantomData,
        }
    }
}

impl<F, T, R> Clone for Handler<F, T, R>
where
    F: Factory<T, R>,
    R: Responder,
{
    fn clone(&self) -> Self {
        Self {
            hnd: self.hnd.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, T, R> Service for Handler<F, T, R>
where
    F: Factory<T, R>,
    R: Responder,
{
    type Request = (T, HttpRequest);
    type Response = ServiceResponse;
    type Error = Infallible;
    type Future = HandlerServiceResponse<R>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (param, req): (T, HttpRequest)) -> Self::Future {
        let fut = self.hnd.call(param).respond_to(&req);
        HandlerServiceResponse {
            fut,
            req: Some(req),
        }
    }
}

#[pin_project]
pub struct HandlerServiceResponse<T: Responder> {
    #[pin]
    fut: T::Future,
    req: Option<HttpRequest>,
}

impl<T: Responder> Future for HandlerServiceResponse<T> {
    type Output = Result<ServiceResponse, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();

        match this.fut.poll(cx) {
            Poll::Ready(Ok(res)) => {
                Poll::Ready(Ok(ServiceResponse::new(this.req.take().unwrap(), res)))
            }
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => {
                let res: Response = e.into().into();
                Poll::Ready(Ok(ServiceResponse::new(this.req.take().unwrap(), res)))
            }
        }
    }
}

/// Async handler converter factory
pub trait AsyncFactory<T, R, O, E>: Clone + 'static
where
    R: Future<Output = Result<O, E>>,
    O: Responder,
    E: Into<Error>,
{
    fn call(&self, param: T) -> R;
}

impl<F, R, O, E> AsyncFactory<(), R, O, E> for F
where
    F: Fn() -> R + Clone + 'static,
    R: Future<Output = Result<O, E>>,
    O: Responder,
    E: Into<Error>,
{
    fn call(&self, _: ()) -> R {
        (self)()
    }
}

#[doc(hidden)]
pub struct AsyncHandler<F, T, R, O, E>
where
    F: AsyncFactory<T, R, O, E>,
    R: Future<Output = Result<O, E>>,
    O: Responder,
    E: Into<Error>,
{
    hnd: F,
    _t: PhantomData<(T, R, O, E)>,
}

impl<F, T, R, O, E> AsyncHandler<F, T, R, O, E>
where
    F: AsyncFactory<T, R, O, E>,
    R: Future<Output = Result<O, E>>,
    O: Responder,
    E: Into<Error>,
{
    pub fn new(hnd: F) -> Self {
        AsyncHandler {
            hnd,
            _t: PhantomData,
        }
    }
}

impl<F, T, R, O, E> Clone for AsyncHandler<F, T, R, O, E>
where
    F: AsyncFactory<T, R, O, E>,
    R: Future<Output = Result<O, E>>,
    O: Responder,
    E: Into<Error>,
{
    fn clone(&self) -> Self {
        AsyncHandler {
            hnd: self.hnd.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, T, R, O, E> Service for AsyncHandler<F, T, R, O, E>
where
    F: AsyncFactory<T, R, O, E>,
    R: Future<Output = Result<O, E>>,
    O: Responder,
    E: Into<Error>,
{
    type Request = (T, HttpRequest);
    type Response = ServiceResponse;
    type Error = Infallible;
    type Future = AsyncHandlerServiceResponse<R, O, E>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (param, req): (T, HttpRequest)) -> Self::Future {
        AsyncHandlerServiceResponse {
            fut: self.hnd.call(param),
            fut2: None,
            req: Some(req),
        }
    }
}

#[doc(hidden)]
#[pin_project]
pub struct AsyncHandlerServiceResponse<T, R, E>
where
    T: Future<Output = Result<R, E>>,
    R: Responder,
    E: Into<Error>,
{
    #[pin]
    fut: T,
    #[pin]
    fut2: Option<R::Future>,
    req: Option<HttpRequest>,
}

impl<T, R, E> Future for AsyncHandlerServiceResponse<T, R, E>
where
    T: Future<Output = Result<R, E>>,
    R: Responder,
    E: Into<Error>,
{
    type Output = Result<ServiceResponse, Infallible>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        if let Some(fut) = this.fut2.as_pin_mut() {
            return match fut.poll(cx) {
                Poll::Ready(Ok(res)) => {
                    Poll::Ready(Ok(ServiceResponse::new(this.req.take().unwrap(), res)))
                }
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(e)) => {
                    let res: Response = e.into().into();
                    Poll::Ready(Ok(ServiceResponse::new(this.req.take().unwrap(), res)))
                }
            };
        }

        match this.fut.poll(cx) {
            Poll::Ready(Ok(res)) => {
                let fut = res.respond_to(this.req.as_ref().unwrap());
                self.as_mut().project().fut2.set(Some(fut));
                self.poll(cx)
            }
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => {
                let res: Response = e.into().into();
                Poll::Ready(Ok(ServiceResponse::new(this.req.take().unwrap(), res)))
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
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = (Error, ServiceRequest);
    type InitError = ();
    type Service = ExtractService<T, S>;
    type Future = Ready<Result<Self::Service, ()>>;

    fn new_service(&self, _: &()) -> Self::Future {
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
    type Error = (Error, ServiceRequest);
    type Future = ExtractResponse<T, S>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let (req, mut payload) = req.into_parts();
        let fut = T::from_request(&req, &mut payload);

        ExtractResponse {
            fut,
            req,
            fut_s: None,
            service: self.service.clone(),
        }
    }
}

#[pin_project]
pub struct ExtractResponse<T: FromRequest, S: Service> {
    req: HttpRequest,
    service: S,
    #[pin]
    fut: T::Future,
    #[pin]
    fut_s: Option<S::Future>,
}

impl<T: FromRequest, S> Future for ExtractResponse<T, S>
where
    S: Service<
        Request = (T, HttpRequest),
        Response = ServiceResponse,
        Error = Infallible,
    >,
{
    type Output = Result<ServiceResponse, (Error, ServiceRequest)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        if let Some(fut) = this.fut_s.as_pin_mut() {
            return fut.poll(cx).map_err(|_| panic!());
        }

        match ready!(this.fut.poll(cx)) {
            Err(e) => {
                let req = ServiceRequest::new(this.req.clone());
                Poll::Ready(Err((e.into(), req)))
            }
            Ok(item) => {
                let fut = Some(this.service.call((item, this.req.clone())));
                self.as_mut().project().fut_s.set(fut);
                self.poll(cx)
            }
        }
    }
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $(($n:tt, $T:ident)),+} => {
    impl<Func, $($T,)+ Res> Factory<($($T,)+), Res> for Func
    where Func: Fn($($T,)+) -> Res + Clone,
          Res: Responder,
    {
        fn call(&self, param: ($($T,)+)) -> Res {
            (self)($(param.$n,)+)
        }
    }

    impl<Func, $($T,)+ Res, O, E1> AsyncFactory<($($T,)+), Res, O, E1> for Func
    where Func: Fn($($T,)+) -> Res + Clone + 'static,
          Res: Future<Output = Result<O, E1>>,
          O: Responder,
          E1: Into<Error>,
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
