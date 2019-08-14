use std::fmt;
use std::marker::PhantomData;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::{http::Method, Error};
use actix_service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};
use log::error;

use crate::app::HttpServiceFactory;
use crate::request::FramedRequest;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct FramedRoute<Io, S, F = (), R = ()> {
    handler: F,
    pattern: String,
    methods: Vec<Method>,
    state: PhantomData<(Io, S, R)>,
}

impl<Io, S> FramedRoute<Io, S> {
    pub fn new(pattern: &str) -> Self {
        FramedRoute {
            handler: (),
            pattern: pattern.to_string(),
            methods: Vec::new(),
            state: PhantomData,
        }
    }

    pub fn get(path: &str) -> FramedRoute<Io, S> {
        FramedRoute::new(path).method(Method::GET)
    }

    pub fn post(path: &str) -> FramedRoute<Io, S> {
        FramedRoute::new(path).method(Method::POST)
    }

    pub fn put(path: &str) -> FramedRoute<Io, S> {
        FramedRoute::new(path).method(Method::PUT)
    }

    pub fn delete(path: &str) -> FramedRoute<Io, S> {
        FramedRoute::new(path).method(Method::DELETE)
    }

    pub fn method(mut self, method: Method) -> Self {
        self.methods.push(method);
        self
    }

    pub fn to<F, R>(self, handler: F) -> FramedRoute<Io, S, F, R>
    where
        F: FnMut(FramedRequest<Io, S>) -> R,
        R: IntoFuture<Item = ()>,
        R::Future: 'static,
        R::Error: fmt::Debug,
    {
        FramedRoute {
            handler,
            pattern: self.pattern,
            methods: self.methods,
            state: PhantomData,
        }
    }
}

impl<Io, S, F, R> HttpServiceFactory for FramedRoute<Io, S, F, R>
where
    Io: AsyncRead + AsyncWrite + 'static,
    F: FnMut(FramedRequest<Io, S>) -> R + Clone,
    R: IntoFuture<Item = ()>,
    R::Future: 'static,
    R::Error: fmt::Display,
{
    type Factory = FramedRouteFactory<Io, S, F, R>;

    fn path(&self) -> &str {
        &self.pattern
    }

    fn create(self) -> Self::Factory {
        FramedRouteFactory {
            handler: self.handler,
            methods: self.methods,
            _t: PhantomData,
        }
    }
}

pub struct FramedRouteFactory<Io, S, F, R> {
    handler: F,
    methods: Vec<Method>,
    _t: PhantomData<(Io, S, R)>,
}

impl<Io, S, F, R> NewService for FramedRouteFactory<Io, S, F, R>
where
    Io: AsyncRead + AsyncWrite + 'static,
    F: FnMut(FramedRequest<Io, S>) -> R + Clone,
    R: IntoFuture<Item = ()>,
    R::Future: 'static,
    R::Error: fmt::Display,
{
    type Config = ();
    type Request = FramedRequest<Io, S>;
    type Response = ();
    type Error = Error;
    type InitError = ();
    type Service = FramedRouteService<Io, S, F, R>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(FramedRouteService {
            handler: self.handler.clone(),
            methods: self.methods.clone(),
            _t: PhantomData,
        })
    }
}

pub struct FramedRouteService<Io, S, F, R> {
    handler: F,
    methods: Vec<Method>,
    _t: PhantomData<(Io, S, R)>,
}

impl<Io, S, F, R> Service for FramedRouteService<Io, S, F, R>
where
    Io: AsyncRead + AsyncWrite + 'static,
    F: FnMut(FramedRequest<Io, S>) -> R + Clone,
    R: IntoFuture<Item = ()>,
    R::Future: 'static,
    R::Error: fmt::Display,
{
    type Request = FramedRequest<Io, S>;
    type Response = ();
    type Error = Error;
    type Future = Box<dyn Future<Item = (), Error = Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: FramedRequest<Io, S>) -> Self::Future {
        Box::new((self.handler)(req).into_future().then(|res| {
            if let Err(e) = res {
                error!("Error in request handler: {}", e);
            }
            Ok(())
        }))
    }
}
