use std::fmt;
use std::marker::PhantomData;

use actix_server_config::ServerConfig as SrvConfig;
use actix_service::{IntoNewService, NewService, Service};

use crate::body::MessageBody;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::Error;
use crate::h1::{ExpectHandler, H1Service};
use crate::h2::H2Service;
use crate::request::Request;
use crate::response::Response;
use crate::service::HttpService;

/// A http service builder
///
/// This type can be used to construct an instance of `http service` through a
/// builder-like pattern.
pub struct HttpServiceBuilder<T, S, X = ExpectHandler> {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_disconnect: u64,
    expect: X,
    _t: PhantomData<(T, S)>,
}

impl<T, S> HttpServiceBuilder<T, S, ExpectHandler>
where
    S: NewService<SrvConfig, Request = Request>,
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
            _t: PhantomData,
        }
    }
}

impl<T, S, X> HttpServiceBuilder<T, S, X>
where
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
{
    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<U: Into<KeepAlive>>(mut self, val: U) -> Self {
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
    pub fn expect<F, U>(self, expect: F) -> HttpServiceBuilder<T, S, U>
    where
        F: IntoNewService<U>,
        U: NewService<Request = Request, Response = Request>,
        U::Error: Into<Error>,
        U::InitError: fmt::Debug,
    {
        HttpServiceBuilder {
            keep_alive: self.keep_alive,
            client_timeout: self.client_timeout,
            client_disconnect: self.client_disconnect,
            expect: expect.into_new_service(),
            _t: PhantomData,
        }
    }

    // #[cfg(feature = "ssl")]
    // /// Configure alpn protocols for SslAcceptorBuilder.
    // pub fn configure_openssl(
    //     builder: &mut openssl::ssl::SslAcceptorBuilder,
    // ) -> io::Result<()> {
    //     let protos: &[u8] = b"\x02h2";
    //     builder.set_alpn_select_callback(|_, protos| {
    //         const H2: &[u8] = b"\x02h2";
    //         if protos.windows(3).any(|window| window == H2) {
    //             Ok(b"h2")
    //         } else {
    //             Err(openssl::ssl::AlpnError::NOACK)
    //         }
    //     });
    //     builder.set_alpn_protos(&protos)?;

    //     Ok(())
    // }

    /// Finish service configuration and create *http service* for HTTP/1 protocol.
    pub fn h1<F, P, B>(self, service: F) -> H1Service<T, P, S, B, X>
    where
        B: MessageBody + 'static,
        F: IntoNewService<S, SrvConfig>,
        S::Error: Into<Error>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
        );
        H1Service::with_config(cfg, service.into_new_service()).expect(self.expect)
    }

    /// Finish service configuration and create *http service* for HTTP/2 protocol.
    pub fn h2<F, P, B>(self, service: F) -> H2Service<T, P, S, B>
    where
        B: MessageBody + 'static,
        F: IntoNewService<S, SrvConfig>,
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
    }

    /// Finish service configuration and create `HttpService` instance.
    pub fn finish<F, P, B>(self, service: F) -> HttpService<T, P, S, B, X>
    where
        B: MessageBody + 'static,
        F: IntoNewService<S, SrvConfig>,
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
        HttpService::with_config(cfg, service.into_new_service()).expect(self.expect)
    }
}
