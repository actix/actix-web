use std::marker::PhantomData;
use std::rc::Rc;
use std::{fmt, net};

use actix_codec::Framed;
use actix_service::{IntoServiceFactory, Service, ServiceFactory};

use crate::body::MessageBody;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::Error;
use crate::h1::{Codec, ExpectHandler, H1Service, UpgradeHandler};
use crate::h2::H2Service;
use crate::helpers::{Data, DataFactory};
use crate::request::Request;
use crate::response::Response;
use crate::service::HttpService;
use crate::{ConnectCallback, Extensions};

/// A HTTP service builder
///
/// This type can be used to construct an instance of [`HttpService`] through a
/// builder-like pattern.
pub struct HttpServiceBuilder<T, S, X = ExpectHandler, U = UpgradeHandler<T>> {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_disconnect: u64,
    secure: bool,
    local_addr: Option<net::SocketAddr>,
    expect: X,
    upgrade: Option<U>,
    // DEPRECATED: in favor of on_connect_ext
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _t: PhantomData<(T, S)>,
}

impl<T, S> HttpServiceBuilder<T, S, ExpectHandler, UpgradeHandler<T>>
where
    S: ServiceFactory<Config = (), Request = Request>,
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    <S::Service as Service>::Future: 'static,
{
    /// Create instance of `ServiceConfigBuilder`
    pub fn new() -> Self {
        HttpServiceBuilder {
            keep_alive: KeepAlive::Timeout(5),
            client_timeout: 5000,
            client_disconnect: 0,
            secure: false,
            local_addr: None,
            expect: ExpectHandler,
            upgrade: None,
            on_connect: None,
            on_connect_ext: None,
            _t: PhantomData,
        }
    }
}

impl<T, S, X, U> HttpServiceBuilder<T, S, X, U>
where
    S: ServiceFactory<Config = (), Request = Request>,
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    <S::Service as Service>::Future: 'static,
    X: ServiceFactory<Config = (), Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    <X::Service as Service>::Future: 'static,
    U: ServiceFactory<Config = (), Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
    <U::Service as Service>::Future: 'static,
{
    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<W: Into<KeepAlive>>(mut self, val: W) -> Self {
        self.keep_alive = val.into();
        self
    }

    /// Set connection secure state
    pub fn secure(mut self) -> Self {
        self.secure = true;
        self
    }

    /// Set the local address that this service is bound to.
    pub fn local_addr(mut self, addr: net::SocketAddr) -> Self {
        self.local_addr = Some(addr);
        self
    }

    /// Set server client timeout in milliseconds for first request.
    ///
    /// Defines a timeout for reading client request header. If a client does not transmit
    /// the entire set headers within this time, the request is terminated with
    /// the 408 (Request Time-out) error.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default client timeout is set to 5000 milliseconds.
    pub fn client_timeout(mut self, val: u64) -> Self {
        self.client_timeout = val;
        self
    }

    /// Set server connection disconnect timeout in milliseconds.
    ///
    /// Defines a timeout for disconnect connection. If a disconnect procedure does not complete
    /// within this time, the request get dropped. This timeout affects secure connections.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default disconnect timeout is set to 0.
    pub fn client_disconnect(mut self, val: u64) -> Self {
        self.client_disconnect = val;
        self
    }

    /// Provide service for `EXPECT: 100-Continue` support.
    ///
    /// Service get called with request that contains `EXPECT` header.
    /// Service must return request in case of success, in that case
    /// request will be forwarded to main service.
    pub fn expect<F, X1>(self, expect: F) -> HttpServiceBuilder<T, S, X1, U>
    where
        F: IntoServiceFactory<X1>,
        X1: ServiceFactory<Config = (), Request = Request, Response = Request>,
        X1::Error: Into<Error>,
        X1::InitError: fmt::Debug,
        <X1::Service as Service>::Future: 'static,
    {
        HttpServiceBuilder {
            keep_alive: self.keep_alive,
            client_timeout: self.client_timeout,
            client_disconnect: self.client_disconnect,
            secure: self.secure,
            local_addr: self.local_addr,
            expect: expect.into_factory(),
            upgrade: self.upgrade,
            on_connect: self.on_connect,
            on_connect_ext: self.on_connect_ext,
            _t: PhantomData,
        }
    }

    /// Provide service for custom `Connection: UPGRADE` support.
    ///
    /// If service is provided then normal requests handling get halted
    /// and this service get called with original request and framed object.
    pub fn upgrade<F, U1>(self, upgrade: F) -> HttpServiceBuilder<T, S, X, U1>
    where
        F: IntoServiceFactory<U1>,
        U1: ServiceFactory<
            Config = (),
            Request = (Request, Framed<T, Codec>),
            Response = (),
        >,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
        <U1::Service as Service>::Future: 'static,
    {
        HttpServiceBuilder {
            keep_alive: self.keep_alive,
            client_timeout: self.client_timeout,
            client_disconnect: self.client_disconnect,
            secure: self.secure,
            local_addr: self.local_addr,
            expect: self.expect,
            upgrade: Some(upgrade.into_factory()),
            on_connect: self.on_connect,
            on_connect_ext: self.on_connect_ext,
            _t: PhantomData,
        }
    }

    /// Set on-connect callback.
    ///
    /// Called once per connection. Return value of the call is stored in request extensions.
    ///
    /// *SOFT DEPRECATED*: Prefer the `on_connect_ext` style callback.
    pub fn on_connect<F, I>(mut self, f: F) -> Self
    where
        F: Fn(&T) -> I + 'static,
        I: Clone + 'static,
    {
        self.on_connect = Some(Rc::new(move |io| Box::new(Data(f(io)))));
        self
    }

    /// Sets the callback to be run on connection establishment.
    ///
    /// Has mutable access to a data container that will be merged into request extensions.
    /// This enables transport layer data (like client certificates) to be accessed in middleware
    /// and handlers.
    pub fn on_connect_ext<F>(mut self, f: F) -> Self
    where
        F: Fn(&T, &mut Extensions) + 'static,
    {
        self.on_connect_ext = Some(Rc::new(f));
        self
    }

    /// Finish service configuration and create a HTTP Service for HTTP/1 protocol.
    pub fn h1<F, B>(self, service: F) -> H1Service<T, S, B, X, U>
    where
        B: MessageBody,
        F: IntoServiceFactory<S>,
        S::Error: Into<Error>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
            self.secure,
            self.local_addr,
        );

        H1Service::with_config(cfg, service.into_factory())
            .expect(self.expect)
            .upgrade(self.upgrade)
            .on_connect(self.on_connect)
            .on_connect_ext(self.on_connect_ext)
    }

    /// Finish service configuration and create a HTTP service for HTTP/2 protocol.
    pub fn h2<F, B>(self, service: F) -> H2Service<T, S, B>
    where
        B: MessageBody + 'static,
        F: IntoServiceFactory<S>,
        S::Error: Into<Error> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service>::Future: 'static,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
            self.secure,
            self.local_addr,
        );

        H2Service::with_config(cfg, service.into_factory())
            .on_connect(self.on_connect)
            .on_connect_ext(self.on_connect_ext)
    }

    /// Finish service configuration and create `HttpService` instance.
    pub fn finish<F, B>(self, service: F) -> HttpService<T, S, B, X, U>
    where
        B: MessageBody + 'static,
        F: IntoServiceFactory<S>,
        S::Error: Into<Error> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service>::Future: 'static,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
            self.secure,
            self.local_addr,
        );

        HttpService::with_config(cfg, service.into_factory())
            .expect(self.expect)
            .upgrade(self.upgrade)
            .on_connect(self.on_connect)
            .on_connect_ext(self.on_connect_ext)
    }
}
