use std::{fmt, marker::PhantomData, net, rc::Rc, time::Duration};

use actix_codec::Framed;
use actix_service::{IntoServiceFactory, Service, ServiceFactory};

use crate::{
    body::{BoxBody, MessageBody},
    h1::{self, ExpectHandler, H1Service, UpgradeHandler},
    service::HttpService,
    ConnectCallback, Extensions, KeepAlive, Request, Response, ServiceConfig,
};

/// An HTTP service builder.
///
/// This type can construct an instance of [`HttpService`] through a builder-like pattern.
pub struct HttpServiceBuilder<T, S, X = ExpectHandler, U = UpgradeHandler> {
    keep_alive: KeepAlive,
    client_request_timeout: Duration,
    client_disconnect_timeout: Duration,
    secure: bool,
    local_addr: Option<net::SocketAddr>,
    expect: X,
    upgrade: Option<U>,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _phantom: PhantomData<S>,
}

impl<T, S> Default for HttpServiceBuilder<T, S, ExpectHandler, UpgradeHandler>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::InitError: fmt::Debug,
    <S::Service as Service<Request>>::Future: 'static,
{
    fn default() -> Self {
        HttpServiceBuilder {
            // ServiceConfig parts (make sure defaults match)
            keep_alive: KeepAlive::default(),
            client_request_timeout: Duration::from_secs(5),
            client_disconnect_timeout: Duration::ZERO,
            secure: false,
            local_addr: None,

            // dispatcher parts
            expect: ExpectHandler,
            upgrade: None,
            on_connect_ext: None,
            _phantom: PhantomData,
        }
    }
}

impl<T, S, X, U> HttpServiceBuilder<T, S, X, U>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::InitError: fmt::Debug,
    <S::Service as Service<Request>>::Future: 'static,
    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Error: Into<Response<BoxBody>>,
    X::InitError: fmt::Debug,
    U: ServiceFactory<(Request, Framed<T, h1::Codec>), Config = (), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
{
    /// Set connection keep-alive setting.
    ///
    /// Applies to HTTP/1.1 keep-alive and HTTP/2 ping-pong.
    ///
    /// By default keep-alive is 5 seconds.
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

    /// Set client request timeout (for first request).
    ///
    /// Defines a timeout for reading client request header. If the client does not transmit the
    /// request head within this duration, the connection is terminated with a `408 Request Timeout`
    /// response error.
    ///
    /// A duration of zero disables the timeout.
    ///
    /// By default, the client timeout is 5 seconds.
    pub fn client_request_timeout(mut self, dur: Duration) -> Self {
        self.client_request_timeout = dur;
        self
    }

    #[doc(hidden)]
    #[deprecated(since = "3.0.0", note = "Renamed to `client_request_timeout`.")]
    pub fn client_timeout(self, dur: Duration) -> Self {
        self.client_request_timeout(dur)
    }

    /// Set client connection disconnect timeout.
    ///
    /// Defines a timeout for disconnect connection. If a disconnect procedure does not complete
    /// within this time, the request get dropped. This timeout affects secure connections.
    ///
    /// A duration of zero disables the timeout.
    ///
    /// By default, the disconnect timeout is disabled.
    pub fn client_disconnect_timeout(mut self, dur: Duration) -> Self {
        self.client_disconnect_timeout = dur;
        self
    }

    #[doc(hidden)]
    #[deprecated(since = "3.0.0", note = "Renamed to `client_disconnect_timeout`.")]
    pub fn client_disconnect(self, dur: Duration) -> Self {
        self.client_disconnect_timeout(dur)
    }

    /// Provide service for `EXPECT: 100-Continue` support.
    ///
    /// Service get called with request that contains `EXPECT` header.
    /// Service must return request in case of success, in that case
    /// request will be forwarded to main service.
    pub fn expect<F, X1>(self, expect: F) -> HttpServiceBuilder<T, S, X1, U>
    where
        F: IntoServiceFactory<X1, Request>,
        X1: ServiceFactory<Request, Config = (), Response = Request>,
        X1::Error: Into<Response<BoxBody>>,
        X1::InitError: fmt::Debug,
    {
        HttpServiceBuilder {
            keep_alive: self.keep_alive,
            client_request_timeout: self.client_request_timeout,
            client_disconnect_timeout: self.client_disconnect_timeout,
            secure: self.secure,
            local_addr: self.local_addr,
            expect: expect.into_factory(),
            upgrade: self.upgrade,
            on_connect_ext: self.on_connect_ext,
            _phantom: PhantomData,
        }
    }

    /// Provide service for custom `Connection: UPGRADE` support.
    ///
    /// If service is provided then normal requests handling get halted
    /// and this service get called with original request and framed object.
    pub fn upgrade<F, U1>(self, upgrade: F) -> HttpServiceBuilder<T, S, X, U1>
    where
        F: IntoServiceFactory<U1, (Request, Framed<T, h1::Codec>)>,
        U1: ServiceFactory<(Request, Framed<T, h1::Codec>), Config = (), Response = ()>,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
    {
        HttpServiceBuilder {
            keep_alive: self.keep_alive,
            client_request_timeout: self.client_request_timeout,
            client_disconnect_timeout: self.client_disconnect_timeout,
            secure: self.secure,
            local_addr: self.local_addr,
            expect: self.expect,
            upgrade: Some(upgrade.into_factory()),
            on_connect_ext: self.on_connect_ext,
            _phantom: PhantomData,
        }
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

    /// Finish service configuration and create a service for the HTTP/1 protocol.
    pub fn h1<F, B>(self, service: F) -> H1Service<T, S, B, X, U>
    where
        B: MessageBody,
        F: IntoServiceFactory<S, Request>,
        S::Error: Into<Response<BoxBody>>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_request_timeout,
            self.client_disconnect_timeout,
            self.secure,
            self.local_addr,
        );

        H1Service::with_config(cfg, service.into_factory())
            .expect(self.expect)
            .upgrade(self.upgrade)
            .on_connect_ext(self.on_connect_ext)
    }

    /// Finish service configuration and create a service for the HTTP/2 protocol.
    #[cfg(feature = "http2")]
    pub fn h2<F, B>(self, service: F) -> crate::h2::H2Service<T, S, B>
    where
        F: IntoServiceFactory<S, Request>,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,

        B: MessageBody + 'static,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_request_timeout,
            self.client_disconnect_timeout,
            self.secure,
            self.local_addr,
        );

        crate::h2::H2Service::with_config(cfg, service.into_factory())
            .on_connect_ext(self.on_connect_ext)
    }

    /// Finish service configuration and create `HttpService` instance.
    pub fn finish<F, B>(self, service: F) -> HttpService<T, S, B, X, U>
    where
        F: IntoServiceFactory<S, Request>,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,

        B: MessageBody + 'static,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_request_timeout,
            self.client_disconnect_timeout,
            self.secure,
            self.local_addr,
        );

        HttpService::with_config(cfg, service.into_factory())
            .expect(self.expect)
            .upgrade(self.upgrade)
            .on_connect_ext(self.on_connect_ext)
    }
}
