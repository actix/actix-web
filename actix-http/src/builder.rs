use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_codec::Framed;
use actix_server_config::ServerConfig as SrvConfig;
use actix_service::{IntoNewService, NewService, Service};

use crate::body::MessageBody;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::Error;
use crate::h1::{Codec, ExpectHandler, H1Service, UpgradeHandler};
use crate::h2::H2Service;
use crate::helpers::{Data, DataFactory};
use crate::request::Request;
use crate::response::Response;
use crate::service::HttpService;

/// A http service builder
///
/// This type can be used to construct an instance of `http service` through a
/// builder-like pattern.
pub struct HttpServiceBuilder<T, S, X = ExpectHandler, U = UpgradeHandler<T>> {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_disconnect: u64,
    expect: X,
    upgrade: Option<U>,
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, S)>,
}

impl<T, S> HttpServiceBuilder<T, S, ExpectHandler, UpgradeHandler<T>>
where
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
{
    /// Create instance of `ServiceConfigBuilder`
    pub fn new() -> Self {
        HttpServiceBuilder {
            keep_alive: KeepAlive::Timeout(5),
            client_timeout: 5000,
            client_disconnect: 0,
            expect: ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }
}

impl<T, S, X, U> HttpServiceBuilder<T, S, X, U>
where
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
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
    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<W: Into<KeepAlive>>(mut self, val: W) -> Self {
        self.keep_alive = val.into();
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
        F: IntoNewService<X1>,
        X1: NewService<Config = SrvConfig, Request = Request, Response = Request>,
        X1::Error: Into<Error>,
        X1::InitError: fmt::Debug,
    {
        HttpServiceBuilder {
            keep_alive: self.keep_alive,
            client_timeout: self.client_timeout,
            client_disconnect: self.client_disconnect,
            expect: expect.into_new_service(),
            upgrade: self.upgrade,
            on_connect: self.on_connect,
            _t: PhantomData,
        }
    }

    /// Provide service for custom `Connection: UPGRADE` support.
    ///
    /// If service is provided then normal requests handling get halted
    /// and this service get called with original request and framed object.
    pub fn upgrade<F, U1>(self, upgrade: F) -> HttpServiceBuilder<T, S, X, U1>
    where
        F: IntoNewService<U1>,
        U1: NewService<
            Config = SrvConfig,
            Request = (Request, Framed<T, Codec>),
            Response = (),
        >,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
    {
        HttpServiceBuilder {
            keep_alive: self.keep_alive,
            client_timeout: self.client_timeout,
            client_disconnect: self.client_disconnect,
            expect: self.expect,
            upgrade: Some(upgrade.into_new_service()),
            on_connect: self.on_connect,
            _t: PhantomData,
        }
    }

    /// Set on-connect callback.
    ///
    /// It get called once per connection and result of the call
    /// get stored to the request's extensions.
    pub fn on_connect<F, I>(mut self, f: F) -> Self
    where
        F: Fn(&T) -> I + 'static,
        I: Clone + 'static,
    {
        self.on_connect = Some(Rc::new(move |io| Box::new(Data(f(io)))));
        self
    }

    /// Finish service configuration and create *http service* for HTTP/1 protocol.
    pub fn h1<F, P, B>(self, service: F) -> H1Service<T, P, S, B, X, U>
    where
        B: MessageBody + 'static,
        F: IntoNewService<S>,
        S::Error: Into<Error>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
        );
        H1Service::with_config(cfg, service.into_new_service())
            .expect(self.expect)
            .upgrade(self.upgrade)
            .on_connect(self.on_connect)
    }

    /// Finish service configuration and create *http service* for HTTP/2 protocol.
    pub fn h2<F, P, B>(self, service: F) -> H2Service<T, P, S, B>
    where
        B: MessageBody + 'static,
        F: IntoNewService<S>,
        S::Error: Into<Error>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
        <S::Service as Service>::Future: 'static,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
        );
        H2Service::with_config(cfg, service.into_new_service())
            .on_connect(self.on_connect)
    }

    /// Finish service configuration and create `HttpService` instance.
    pub fn finish<F, P, B>(self, service: F) -> HttpService<T, P, S, B, X, U>
    where
        B: MessageBody + 'static,
        F: IntoNewService<S>,
        S::Error: Into<Error>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
        <S::Service as Service>::Future: 'static,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
        );
        HttpService::with_config(cfg, service.into_new_service())
            .expect(self.expect)
            .upgrade(self.upgrade)
            .on_connect(self.on_connect)
    }
}
