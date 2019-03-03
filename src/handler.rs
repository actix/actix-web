use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::{Error, Extensions, Response};
use actix_service::{NewService, Service, Void};
use futures::future::{ok, FutureResult};
use futures::{try_ready, Async, Future, IntoFuture, Poll};

use crate::request::HttpRequest;
use crate::responder::Responder;
use crate::service::{ServiceFromRequest, ServiceRequest, ServiceResponse};

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest<P>: Sized {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Future that resolves to a Self
    type Future: Future<Item = Self, Error = Self::Error>;

    /// Configuration for the extractor
    type Config: ExtractorConfig;

    /// Convert request to a Self
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future;
}

/// Storage for extractor configs
#[derive(Default)]
pub struct ConfigStorage {
    pub(crate) storage: Option<Rc<Extensions>>,
}

impl ConfigStorage {
    pub fn store<C: ExtractorConfig>(&mut self, config: C) {
        if self.storage.is_none() {
            self.storage = Some(Rc::new(Extensions::new()));
        }
        if let Some(ref mut ext) = self.storage {
            Rc::get_mut(ext).unwrap().insert(config);
        }
    }
}

pub trait ExtractorConfig: Default + Clone + 'static {
    /// Set default configuration to config storage
    fn store_default(ext: &mut ConfigStorage) {
        ext.store(Self::default())
    }
}

impl ExtractorConfig for () {
    fn store_default(_: &mut ConfigStorage) {}
}

/// Handler converter factory
pub trait Factory<T, R>: Clone
where
    R: Responder,
{
    fn call(&self, param: T) -> R;
}

impl<F, R> Factory<(), R> for F
where
    F: Fn() -> R + Clone + 'static,
    R: Responder + 'static,
{
    fn call(&self, _: ()) -> R {
        (self)()
    }
}

#[doc(hidden)]
pub struct Handle<F, T, R>
where
    F: Factory<T, R>,
    R: Responder,
{
    hnd: F,
    _t: PhantomData<(T, R)>,
}

impl<F, T, R> Handle<F, T, R>
where
    F: Factory<T, R>,
    R: Responder,
{
    pub fn new(hnd: F) -> Self {
        Handle {
            hnd,
            _t: PhantomData,
        }
    }
}
impl<F, T, R> NewService for Handle<F, T, R>
where
    F: Factory<T, R>,
    R: Responder + 'static,
{
    type Request = (T, HttpRequest);
    type Response = ServiceResponse;
    type Error = Void;
    type InitError = ();
    type Service = HandleService<F, T, R>;
    type Future = FutureResult<Self::Service, ()>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(HandleService {
            hnd: self.hnd.clone(),
            _t: PhantomData,
        })
    }
}

#[doc(hidden)]
pub struct HandleService<F, T, R>
where
    F: Factory<T, R>,
    R: Responder + 'static,
{
    hnd: F,
    _t: PhantomData<(T, R)>,
}

impl<F, T, R> Service for HandleService<F, T, R>
where
    F: Factory<T, R>,
    R: Responder + 'static,
{
    type Request = (T, HttpRequest);
    type Response = ServiceResponse;
    type Error = Void;
    type Future = HandleServiceResponse<R::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (param, req): (T, HttpRequest)) -> Self::Future {
        let fut = self.hnd.call(param).respond_to(&req);
        HandleServiceResponse {
            fut,
            req: Some(req),
        }
    }
}

pub struct HandleServiceResponse<T> {
    fut: T,
    req: Option<HttpRequest>,
}

impl<T> Future for HandleServiceResponse<T>
where
    T: Future<Item = Response>,
    T::Error: Into<Error>,
{
    type Item = ServiceResponse;
    type Error = Void;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::Ready(res)) => Ok(Async::Ready(ServiceResponse::new(
                self.req.take().unwrap(),
                res,
            ))),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                let res: Response = e.into().into();
                Ok(Async::Ready(ServiceResponse::new(
                    self.req.take().unwrap(),
                    res,
                )))
            }
        }
    }
}

/// Async handler converter factory
pub trait AsyncFactory<T, R>: Clone + 'static
where
    R: IntoFuture,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    fn call(&self, param: T) -> R;
}

impl<F, R> AsyncFactory<(), R> for F
where
    F: Fn() -> R + Clone + 'static,
    R: IntoFuture,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    fn call(&self, _: ()) -> R {
        (self)()
    }
}

#[doc(hidden)]
pub struct AsyncHandle<F, T, R>
where
    F: AsyncFactory<T, R>,
    R: IntoFuture,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    hnd: F,
    _t: PhantomData<(T, R)>,
}

impl<F, T, R> AsyncHandle<F, T, R>
where
    F: AsyncFactory<T, R>,
    R: IntoFuture,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    pub fn new(hnd: F) -> Self {
        AsyncHandle {
            hnd,
            _t: PhantomData,
        }
    }
}
impl<F, T, R> NewService for AsyncHandle<F, T, R>
where
    F: AsyncFactory<T, R>,
    R: IntoFuture,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    type Request = (T, HttpRequest);
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = AsyncHandleService<F, T, R>;
    type Future = FutureResult<Self::Service, ()>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(AsyncHandleService {
            hnd: self.hnd.clone(),
            _t: PhantomData,
        })
    }
}

#[doc(hidden)]
pub struct AsyncHandleService<F, T, R>
where
    F: AsyncFactory<T, R>,
    R: IntoFuture,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    hnd: F,
    _t: PhantomData<(T, R)>,
}

impl<F, T, R> Service for AsyncHandleService<F, T, R>
where
    F: AsyncFactory<T, R>,
    R: IntoFuture,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    type Request = (T, HttpRequest);
    type Response = ServiceResponse;
    type Error = ();
    type Future = AsyncHandleServiceResponse<R::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (param, req): (T, HttpRequest)) -> Self::Future {
        AsyncHandleServiceResponse {
            fut: self.hnd.call(param).into_future(),
            req: Some(req),
        }
    }
}

#[doc(hidden)]
pub struct AsyncHandleServiceResponse<T> {
    fut: T,
    req: Option<HttpRequest>,
}

impl<T> Future for AsyncHandleServiceResponse<T>
where
    T: Future,
    T::Item: Into<Response>,
    T::Error: Into<Error>,
{
    type Item = ServiceResponse;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::Ready(res)) => Ok(Async::Ready(ServiceResponse::new(
                self.req.take().unwrap(),
                res.into(),
            ))),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                let res: Response = e.into().into();
                Ok(Async::Ready(ServiceResponse::new(
                    self.req.take().unwrap(),
                    res,
                )))
            }
        }
    }
}

/// Extract arguments from request
pub struct Extract<P, T: FromRequest<P>> {
    config: Rc<RefCell<Option<Rc<Extensions>>>>,
    _t: PhantomData<(P, T)>,
}

impl<P, T: FromRequest<P>> Extract<P, T> {
    pub fn new(config: Rc<RefCell<Option<Rc<Extensions>>>>) -> Self {
        Extract {
            config,
            _t: PhantomData,
        }
    }
}

impl<P, T: FromRequest<P>> NewService for Extract<P, T> {
    type Request = ServiceRequest<P>;
    type Response = (T, HttpRequest);
    type Error = (Error, ServiceFromRequest<P>);
    type InitError = ();
    type Service = ExtractService<P, T>;
    type Future = FutureResult<Self::Service, ()>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(ExtractService {
            _t: PhantomData,
            config: self.config.borrow().clone(),
        })
    }
}

pub struct ExtractService<P, T: FromRequest<P>> {
    config: Option<Rc<Extensions>>,
    _t: PhantomData<(P, T)>,
}

impl<P, T: FromRequest<P>> Service for ExtractService<P, T> {
    type Request = ServiceRequest<P>;
    type Response = (T, HttpRequest);
    type Error = (Error, ServiceFromRequest<P>);
    type Future = ExtractResponse<P, T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: ServiceRequest<P>) -> Self::Future {
        let mut req = ServiceFromRequest::new(req, self.config.clone());
        ExtractResponse {
            fut: T::from_request(&mut req),
            req: Some(req),
        }
    }
}

pub struct ExtractResponse<P, T: FromRequest<P>> {
    req: Option<ServiceFromRequest<P>>,
    fut: T::Future,
}

impl<P, T: FromRequest<P>> Future for ExtractResponse<P, T> {
    type Item = (T, HttpRequest);
    type Error = (Error, ServiceFromRequest<P>);

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let item = try_ready!(self
            .fut
            .poll()
            .map_err(|e| (e.into(), self.req.take().unwrap())));

        let req = self.req.take().unwrap();
        let req = req.into_request();

        Ok(Async::Ready((item, req)))
    }
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $(($n:tt, $T:ident)),+} => {
    impl<Func, $($T,)+ Res> Factory<($($T,)+), Res> for Func
    where Func: Fn($($T,)+) -> Res + Clone + 'static,
          Res: Responder + 'static,
    {
        fn call(&self, param: ($($T,)+)) -> Res {
            (self)($(param.$n,)+)
        }
    }

    impl<Func, $($T,)+ Res> AsyncFactory<($($T,)+), Res> for Func
    where Func: Fn($($T,)+) -> Res + Clone + 'static,
          Res: IntoFuture + 'static,
          Res::Item: Into<Response>,
          Res::Error: Into<Error>,
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
