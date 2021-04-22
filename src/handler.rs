use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http::Error;
use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ready, Ready};
use futures_core::ready;
use pin_project::pin_project;

use crate::{
    extract::FromRequest,
    request::HttpRequest,
    responder::Responder,
    response::HttpResponse,
    service::{ServiceRequest, ServiceResponse},
};

/// A request handler is an async function that accepts zero or more parameters that can be
/// extracted from a request (i.e., [`impl FromRequest`](crate::FromRequest)) and returns a type
/// that can be converted into an [`HttpResponse`] (that is, it impls the [`Responder`] trait).
///
/// If you got the error `the trait Handler<_, _, _> is not implemented`, then your function is not
/// a valid handler. See [Request Handlers](https://actix.rs/docs/handlers/) for more information.
pub trait Handler<T, R>: Clone + 'static
where
    R: Future,
    R::Output: Responder,
{
    fn call(&self, param: T) -> R;
}

#[doc(hidden)]
/// Extract arguments from request, run factory function and make response.
pub struct HandlerService<F, T, R>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
    hnd: F,
    _phantom: PhantomData<(T, R)>,
}

impl<F, T, R> HandlerService<F, T, R>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
    pub fn new(hnd: F) -> Self {
        Self {
            hnd,
            _phantom: PhantomData,
        }
    }
}

impl<F, T, R> Clone for HandlerService<F, T, R>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
    fn clone(&self) -> Self {
        Self {
            hnd: self.hnd.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<F, T, R> ServiceFactory<ServiceRequest> for HandlerService<F, T, R>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
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

/// HandlerService is both it's ServiceFactory and Service Type.
impl<F, T, R> Service<ServiceRequest> for HandlerService<F, T, R>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
    type Response = ServiceResponse;
    type Error = Error;
    type Future = HandlerServiceFuture<F, T, R>;

    actix_service::always_ready!();

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let (req, mut payload) = req.into_parts();
        let fut = T::from_request(&req, &mut payload);
        HandlerServiceFuture::Extract(fut, Some(req), self.hnd.clone())
    }
}

#[doc(hidden)]
#[pin_project(project = HandlerProj)]
pub enum HandlerServiceFuture<F, T, R>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
    Extract(#[pin] T::Future, Option<HttpRequest>, F),
    Handle(#[pin] R, Option<HttpRequest>),
}

impl<F, T, R> Future for HandlerServiceFuture<F, T, R>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
    // Error type in this future is a placeholder type.
    // all instances of error must be converted to ServiceResponse and return in Ok.
    type Output = Result<ServiceResponse, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                HandlerProj::Extract(fut, req, handle) => {
                    match ready!(fut.poll(cx)) {
                        Ok(item) => {
                            let fut = handle.call(item);
                            let state = HandlerServiceFuture::Handle(fut, req.take());
                            self.as_mut().set(state);
                        }
                        Err(err) => {
                            let req = req.take().unwrap();
                            let res = HttpResponse::from_error(err.into());
                            return Poll::Ready(Ok(ServiceResponse::new(req, res)));
                        }
                    };
                }
                HandlerProj::Handle(fut, req) => {
                    let res = ready!(fut.poll(cx));
                    let req = req.take().unwrap();
                    let res = res.respond_to(&req);
                    return Poll::Ready(Ok(ServiceResponse::new(req, res)));
                }
            }
        }
    }
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $($param:ident)* } => {
    impl<Func, $($param,)* Res> Handler<($($param,)*), Res> for Func
    where Func: Fn($($param),*) -> Res + Clone + 'static,
          Res: Future,
          Res::Output: Responder,
    {
        #[allow(non_snake_case)]
        fn call(&self, ($($param,)*): ($($param,)*)) -> Res {
            (self)($($param,)*)
        }
    }
});

factory_tuple! {}
factory_tuple! { A }
factory_tuple! { A B }
factory_tuple! { A B C }
factory_tuple! { A B C D }
factory_tuple! { A B C D E }
factory_tuple! { A B C D E F }
factory_tuple! { A B C D E F G }
factory_tuple! { A B C D E F G H }
factory_tuple! { A B C D E F G H I }
factory_tuple! { A B C D E F G H I J }
factory_tuple! { A B C D E F G H I J K }
factory_tuple! { A B C D E F G H I J K L }
