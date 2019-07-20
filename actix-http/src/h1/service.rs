use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_codec::Framed;
use actix_server_config::{Io, IoStream, ServerConfig as SrvConfig};
use actix_service::{IntoNewService, NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{try_ready, Async, Future, IntoFuture, Poll, Stream};

use crate::body::MessageBody;
use crate::cloneable::CloneableService;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, Error, ParseError};
use crate::helpers::DataFactory;
use crate::request::Request;
use crate::response::Response;

use super::codec::Codec;
use super::dispatcher::Dispatcher;
use super::{ExpectHandler, Message, UpgradeHandler};

/// `NewService` implementation for HTTP1 transport
pub struct H1Service<T, P, S, B, X = ExpectHandler, U = UpgradeHandler<T>> {
    srv: S,
    cfg: ServiceConfig,
    expect: X,
    upgrade: Option<U>,
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> H1Service<T, P, S, B>
where
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    /// Create new `HttpService` instance with default config.
    pub fn new<F: IntoNewService<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H1Service {
            cfg,
            srv: service.into_new_service(),
            expect: ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub fn with_config<F: IntoNewService<S>>(cfg: ServiceConfig, service: F) -> Self {
        H1Service {
            cfg,
            srv: service.into_new_service(),
            expect: ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X, U> H1Service<T, P, S, B, X, U>
where
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
{
    pub fn expect<X1>(self, expect: X1) -> H1Service<T, P, S, B, X1, U>
    where
        X1: NewService<Request = Request, Response = Request>,
        X1::Error: Into<Error>,
        X1::InitError: fmt::Debug,
    {
        H1Service {
            expect,
            cfg: self.cfg,
            srv: self.srv,
            upgrade: self.upgrade,
            on_connect: self.on_connect,
            _t: PhantomData,
        }
    }

    pub fn upgrade<U1>(self, upgrade: Option<U1>) -> H1Service<T, P, S, B, X, U1>
    where
        U1: NewService<Request = (Request, Framed<T, Codec>), Response = ()>,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
    {
        H1Service {
            upgrade,
            cfg: self.cfg,
            srv: self.srv,
            expect: self.expect,
            on_connect: self.on_connect,
            _t: PhantomData,
        }
    }

    /// Set on connect callback.
    pub(crate) fn on_connect(
        mut self,
        f: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    ) -> Self {
        self.on_connect = f;
        self
    }
}

impl<T, P, S, B, X, U> NewService for H1Service<T, P, S, B, X, U>
where
    T: IoStream,
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
    X: NewService<Config = SrvConfig, Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    U: NewService<
        Config = SrvConfig,
        Request = (Request, Framed<T, Codec>),
        Response = (),
    >,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
{
    type Config = SrvConfig;
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type InitError = ();
    type Service = H1ServiceHandler<T, P, S::Service, B, X::Service, U::Service>;
    type Future = H1ServiceResponse<T, P, S, B, X, U>;

    fn new_service(&self, cfg: &SrvConfig) -> Self::Future {
        H1ServiceResponse {
            fut: self.srv.new_service(cfg).into_future(),
            fut_ex: Some(self.expect.new_service(cfg)),
            fut_upg: self.upgrade.as_ref().map(|f| f.new_service(cfg)),
            expect: None,
            upgrade: None,
            on_connect: self.on_connect.clone(),
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct H1ServiceResponse<T, P, S, B, X, U>
where
    S: NewService<Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    U: NewService<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
{
    fut: S::Future,
    fut_ex: Option<X::Future>,
    fut_upg: Option<U::Future>,
    expect: Option<X::Service>,
    upgrade: Option<U::Service>,
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B, X, U> Future for H1ServiceResponse<T, P, S, B, X, U>
where
    T: IoStream,
    S: NewService<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    U: NewService<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
{
    type Item = H1ServiceHandler<T, P, S::Service, B, X::Service, U::Service>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_ex {
            let expect = try_ready!(fut
                .poll()
                .map_err(|e| log::error!("Init http service error: {:?}", e)));
            self.expect = Some(expect);
            self.fut_ex.take();
        }

        if let Some(ref mut fut) = self.fut_upg {
            let upgrade = try_ready!(fut
                .poll()
                .map_err(|e| log::error!("Init http service error: {:?}", e)));
            self.upgrade = Some(upgrade);
            self.fut_ex.take();
        }

        let service = try_ready!(self
            .fut
            .poll()
            .map_err(|e| log::error!("Init http service error: {:?}", e)));
        Ok(Async::Ready(H1ServiceHandler::new(
            self.cfg.take().unwrap(),
            service,
            self.expect.take().unwrap(),
            self.upgrade.take(),
            self.on_connect.clone(),
        )))
    }
}

/// `Service` implementation for HTTP1 transport
pub struct H1ServiceHandler<T, P, S, B, X, U> {
    srv: CloneableService<S>,
    expect: CloneableService<X>,
    upgrade: Option<CloneableService<U>>,
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    cfg: ServiceConfig,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B, X, U> H1ServiceHandler<T, P, S, B, X, U>
where
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    fn new(
        cfg: ServiceConfig,
        srv: S,
        expect: X,
        upgrade: Option<U>,
        on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    ) -> H1ServiceHandler<T, P, S, B, X, U> {
        H1ServiceHandler {
            srv: CloneableService::new(srv),
            expect: CloneableService::new(expect),
            upgrade: upgrade.map(CloneableService::new),
            cfg,
            on_connect,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X, U> Service for H1ServiceHandler<T, P, S, B, X, U>
where
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = Dispatcher<T, S, B, X, U>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        let ready = self
            .expect
            .poll_ready()
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready();

        let ready = self
            .srv
            .poll_ready()
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready()
            && ready;

        if ready {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let io = req.into_parts().0;

        let on_connect = if let Some(ref on_connect) = self.on_connect {
            Some(on_connect(&io))
        } else {
            None
        };

        Dispatcher::new(
            io,
            self.cfg.clone(),
            self.srv.clone(),
            self.expect.clone(),
            self.upgrade.clone(),
            on_connect,
        )
    }
}

/// `NewService` implementation for `OneRequestService` service
#[derive(Default)]
pub struct OneRequest<T, P> {
    config: ServiceConfig,
    _t: PhantomData<(T, P)>,
}

impl<T, P> OneRequest<T, P>
where
    T: IoStream,
{
    /// Create new `H1SimpleService` instance.
    pub fn new() -> Self {
        OneRequest {
            config: ServiceConfig::default(),
            _t: PhantomData,
        }
    }
}

impl<T, P> NewService for OneRequest<T, P>
where
    T: IoStream,
{
    type Config = SrvConfig;
    type Request = Io<T, P>;
    type Response = (Request, Framed<T, Codec>);
    type Error = ParseError;
    type InitError = ();
    type Service = OneRequestService<T, P>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &SrvConfig) -> Self::Future {
        ok(OneRequestService {
            config: self.config.clone(),
            _t: PhantomData,
        })
    }
}

/// `Service` implementation for HTTP1 transport. Reads one request and returns
/// request and framed object.
pub struct OneRequestService<T, P> {
    config: ServiceConfig,
    _t: PhantomData<(T, P)>,
}

impl<T, P> Service for OneRequestService<T, P>
where
    T: IoStream,
{
    type Request = Io<T, P>;
    type Response = (Request, Framed<T, Codec>);
    type Error = ParseError;
    type Future = OneRequestServiceResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        OneRequestServiceResponse {
            framed: Some(Framed::new(
                req.into_parts().0,
                Codec::new(self.config.clone()),
            )),
        }
    }
}

#[doc(hidden)]
pub struct OneRequestServiceResponse<T>
where
    T: IoStream,
{
    framed: Option<Framed<T, Codec>>,
}

impl<T> Future for OneRequestServiceResponse<T>
where
    T: IoStream,
{
    type Item = (Request, Framed<T, Codec>);
    type Error = ParseError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.framed.as_mut().unwrap().poll()? {
            Async::Ready(Some(req)) => match req {
                Message::Item(req) => {
                    Ok(Async::Ready((req, self.framed.take().unwrap())))
                }
                Message::Chunk(_) => unreachable!("Something is wrong"),
            },
            Async::Ready(None) => Err(ParseError::Incomplete),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
