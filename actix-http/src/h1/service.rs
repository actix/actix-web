use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_codec::Framed;
use actix_server_config::{Io, IoStream, ServerConfig as SrvConfig};
use actix_service::{IntoServiceFactory, Service, ServiceFactory};
use futures::future::{ok, Ready};
use futures::{ready, Stream};

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

/// `ServiceFactory` implementation for HTTP1 transport
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
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin,
    B: MessageBody,
    P: Unpin,
{
    /// Create new `HttpService` instance with default config.
    pub fn new<F: IntoServiceFactory<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H1Service {
            cfg,
            srv: service.into_factory(),
            expect: ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub fn with_config<F: IntoServiceFactory<S>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        H1Service {
            cfg,
            srv: service.into_factory(),
            expect: ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X, U> H1Service<T, P, S, B, X, U>
where
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin,
    B: MessageBody,
    P: Unpin,
{
    pub fn expect<X1>(self, expect: X1) -> H1Service<T, P, S, B, X1, U>
    where
        X1: ServiceFactory<Request = Request, Response = Request>,
        X1::Error: Into<Error>,
        X1::InitError: fmt::Debug,
        X1::Future: Unpin,
        X1::Service: Unpin,
        <X1::Service as Service>::Future: Unpin,
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
        U1: ServiceFactory<Request = (Request, Framed<T, Codec>), Response = ()>,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
        U1::Future: Unpin,
        U1::Service: Unpin,
        <U1::Service as Service>::Future: Unpin,
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

impl<T, P, S, B, X, U> ServiceFactory for H1Service<T, P, S, B, X, U>
where
    T: IoStream,
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Service: Unpin,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin,
    B: MessageBody,
    X: ServiceFactory<Config = SrvConfig, Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    X::Future: Unpin,
    X::Service: Unpin,
    <X::Service as Service>::Future: Unpin,
    U: ServiceFactory<
        Config = SrvConfig,
        Request = (Request, Framed<T, Codec>),
        Response = (),
    >,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
    U::Future: Unpin,
    U::Service: Unpin,
    <U::Service as Service>::Future: Unpin,
    P: Unpin,
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
            fut: self.srv.new_service(cfg),
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
    S: ServiceFactory<Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin,
    X: ServiceFactory<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    X::Future: Unpin,
    X::Service: Unpin,
    <X::Service as Service>::Future: Unpin,
    U: ServiceFactory<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
    U::Future: Unpin,
    U::Service: Unpin,
    <U::Service as Service>::Future: Unpin,
    P: Unpin,
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
    S: ServiceFactory<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin,
    B: MessageBody,
    X: ServiceFactory<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    X::Future: Unpin,
    X::Service: Unpin,
    <X::Service as Service>::Future: Unpin,
    U: ServiceFactory<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
    U::Future: Unpin,
    U::Service: Unpin,
    <U::Service as Service>::Future: Unpin,
    P: Unpin,
{
    type Output =
        Result<H1ServiceHandler<T, P, S::Service, B, X::Service, U::Service>, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();

        if let Some(ref mut fut) = this.fut_ex {
            let expect = ready!(Pin::new(fut)
                .poll(cx)
                .map_err(|e| log::error!("Init http service error: {:?}", e)))?;
            this.expect = Some(expect);
            this.fut_ex.take();
        }

        if let Some(ref mut fut) = this.fut_upg {
            let upgrade = ready!(Pin::new(fut)
                .poll(cx)
                .map_err(|e| log::error!("Init http service error: {:?}", e)))?;
            this.upgrade = Some(upgrade);
            this.fut_ex.take();
        }

        let result = ready!(Pin::new(&mut this.fut)
            .poll(cx)
            .map_err(|e| log::error!("Init http service error: {:?}", e)));

        Poll::Ready(result.map(|service| {
            H1ServiceHandler::new(
                this.cfg.take().unwrap(),
                service,
                this.expect.take().unwrap(),
                this.upgrade.take(),
                this.on_connect.clone(),
            )
        }))
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
    S: Service<Request = Request> + Unpin,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::Future: Unpin,
    B: MessageBody,
    X: Service<Request = Request, Response = Request> + Unpin,
    X::Future: Unpin,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()> + Unpin,
    U::Future: Unpin,
    U::Error: fmt::Display,
    P: Unpin,
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
    S: Service<Request = Request> + Unpin,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::Future: Unpin,
    B: MessageBody,
    X: Service<Request = Request, Response = Request> + Unpin,
    X::Error: Into<Error>,
    X::Future: Unpin,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()> + Unpin,
    U::Error: fmt::Display,
    U::Future: Unpin,
    P: Unpin,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = Dispatcher<T, S, B, X, U>;

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let ready = self
            .expect
            .poll_ready(cx)
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready();

        let ready = self
            .srv
            .poll_ready(cx)
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready()
            && ready;

        if ready {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
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

/// `ServiceFactory` implementation for `OneRequestService` service
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

impl<T, P> ServiceFactory for OneRequest<T, P>
where
    T: IoStream,
{
    type Config = SrvConfig;
    type Request = Io<T, P>;
    type Response = (Request, Framed<T, Codec>);
    type Error = ParseError;
    type InitError = ();
    type Service = OneRequestService<T, P>;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

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

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
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
    type Output = Result<(Request, Framed<T, Codec>), ParseError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match self.framed.as_mut().unwrap().next_item(cx) {
            Poll::Ready(Some(Ok(req))) => match req {
                Message::Item(req) => {
                    Poll::Ready(Ok((req, self.framed.take().unwrap())))
                }
                Message::Chunk(_) => unreachable!("Something is wrong"),
            },
            Poll::Ready(Some(Err(err))) => Poll::Ready(Err(err)),
            Poll::Ready(None) => Poll::Ready(Err(ParseError::Incomplete)),
            Poll::Pending => Poll::Pending,
        }
    }
}
