use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::{http::Method, Error};
use actix_service::{Service, ServiceFactory};
use futures::future::{ok, FutureExt, LocalBoxFuture, Ready};
use log::error;

use crate::app::HttpServiceFactory;
use crate::request::FramedRequest;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct FramedRoute<Io, S, F = (), R = (), E = ()> {
    handler: F,
    pattern: String,
    methods: Vec<Method>,
    state: PhantomData<(Io, S, R, E)>,
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

    pub fn to<F, R, E>(self, handler: F) -> FramedRoute<Io, S, F, R, E>
    where
        F: FnMut(FramedRequest<Io, S>) -> R,
        R: Future<Output = Result<(), E>> + 'static,

        E: fmt::Debug,
    {
        FramedRoute {
            handler,
            pattern: self.pattern,
            methods: self.methods,
            state: PhantomData,
        }
    }
}

impl<Io, S, F, R, E> HttpServiceFactory for FramedRoute<Io, S, F, R, E>
where
    Io: AsyncRead + AsyncWrite + 'static,
    F: FnMut(FramedRequest<Io, S>) -> R + Clone,
    R: Future<Output = Result<(), E>> + 'static,
    E: fmt::Display,
{
    type Factory = FramedRouteFactory<Io, S, F, R, E>;

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

pub struct FramedRouteFactory<Io, S, F, R, E> {
    handler: F,
    methods: Vec<Method>,
    _t: PhantomData<(Io, S, R, E)>,
}

impl<Io, S, F, R, E> ServiceFactory for FramedRouteFactory<Io, S, F, R, E>
where
    Io: AsyncRead + AsyncWrite + 'static,
    F: FnMut(FramedRequest<Io, S>) -> R + Clone,
    R: Future<Output = Result<(), E>> + 'static,
    E: fmt::Display,
{
    type Config = ();
    type Request = FramedRequest<Io, S>;
    type Response = ();
    type Error = Error;
    type InitError = ();
    type Service = FramedRouteService<Io, S, F, R, E>;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(FramedRouteService {
            handler: self.handler.clone(),
            methods: self.methods.clone(),
            _t: PhantomData,
        })
    }
}

pub struct FramedRouteService<Io, S, F, R, E> {
    handler: F,
    methods: Vec<Method>,
    _t: PhantomData<(Io, S, R, E)>,
}

impl<Io, S, F, R, E> Service for FramedRouteService<Io, S, F, R, E>
where
    Io: AsyncRead + AsyncWrite + 'static,
    F: FnMut(FramedRequest<Io, S>) -> R + Clone,
    R: Future<Output = Result<(), E>> + 'static,
    E: fmt::Display,
{
    type Request = FramedRequest<Io, S>;
    type Response = ();
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<(), Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: FramedRequest<Io, S>) -> Self::Future {
        let fut = (self.handler)(req);

        async move {
            let res = fut.await;
            if let Err(e) = res {
                error!("Error in request handler: {}", e);
            }
            Ok(())
        }
            .boxed_local()
    }
}
