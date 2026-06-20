use std::{
    future::Future,
    io, net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::Framed;
use actix_http::{h1::ClientCodec, Payload, RequestHead, RequestHeadType, ResponseHead};
use actix_service::Service;
use bytes::BytesMut;
use futures_core::{future::LocalBoxFuture, ready};
use http::header::HeaderValue;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    any_body::AnyBody,
    client::{Connect as ClientConnect, ConnectError, Connection, ConnectionIo, SendRequestError},
    proxy::Proxy,
    ClientResponse,
};

pub type BoxConnectorService = Rc<
    dyn Service<
        ConnectRequest,
        Response = ConnectResponse,
        Error = SendRequestError,
        Future = LocalBoxFuture<'static, Result<ConnectResponse, SendRequestError>>,
    >,
>;

pub type BoxedSocket = Box<dyn ConnectionIo>;

/// Combined HTTP and WebSocket request type received by connection service.
pub enum ConnectRequest {
    /// Standard HTTP request.
    ///
    /// Contains the request head, body type, and optional pre-resolved socket address.
    Client(RequestHeadType, AnyBody, Option<net::SocketAddr>),

    /// Tunnel used by WebSocket connection requests.
    ///
    /// Contains the request head and optional pre-resolved socket address.
    Tunnel(RequestHead, Option<net::SocketAddr>),
}

/// Combined HTTP response & WebSocket tunnel type returned from connection service.
pub enum ConnectResponse {
    /// Standard HTTP response.
    Client(ClientResponse),

    /// Tunnel used for WebSocket communication.
    ///
    /// Contains response head and framed HTTP/1.1 codec.
    Tunnel(ResponseHead, Framed<BoxedSocket, ClientCodec>),
}

impl ConnectResponse {
    /// Unwraps type into HTTP response.
    ///
    /// # Panics
    /// Panics if enum variant is not `Client`.
    pub fn into_client_response(self) -> ClientResponse {
        match self {
            ConnectResponse::Client(res) => res,
            _ => {
                panic!("ClientResponse only reachable with ConnectResponse::ClientResponse variant")
            }
        }
    }

    /// Unwraps type into WebSocket tunnel response.
    ///
    /// # Panics
    /// Panics if enum variant is not `Tunnel`.
    pub fn into_tunnel_response(self) -> (ResponseHead, Framed<BoxedSocket, ClientCodec>) {
        match self {
            ConnectResponse::Tunnel(head, framed) => (head, framed),
            _ => {
                panic!("TunnelResponse only reachable with ConnectResponse::TunnelResponse variant")
            }
        }
    }
}

pub struct DefaultConnector<S> {
    connector: S,
    proxy: Option<Proxy>,
}

impl<S> DefaultConnector<S> {
    pub(crate) fn new(connector: S, proxy: Option<Proxy>) -> Self {
        Self { connector, proxy }
    }
}

impl<S, Io> Service<ConnectRequest> for DefaultConnector<S>
where
    S: Service<ClientConnect, Error = ConnectError, Response = Connection<Io>>,
    Io: ConnectionIo,
{
    type Response = ConnectResponse;
    type Error = SendRequestError;
    type Future = ConnectRequestFuture<S::Future, Io>;

    actix_service::forward_ready!(connector);

    fn call(&self, req: ConnectRequest) -> Self::Future {
        // Determine the target URI and whether we need a CONNECT tunnel.
        let (target_uri, need_tunnel, tunnel_host) = match &req {
            ConnectRequest::Client(head, ..) => {
                let uri = &head.as_ref().uri;
                let is_https = uri.scheme_str() == Some("https");
                let host_port = format!(
                    "{}:{}",
                    uri.host().unwrap_or(""),
                    uri.port_u16().unwrap_or(if is_https { 443 } else { 80 })
                );
                (uri.clone(), is_https, host_port)
            }
            ConnectRequest::Tunnel(head, _) => {
                let uri = &head.uri;
                let is_https = uri.scheme_str() == Some("https");
                let host_port = format!(
                    "{}:{}",
                    uri.host().unwrap_or(""),
                    uri.port_u16().unwrap_or(if is_https { 443 } else { 80 })
                );
                (uri.clone(), is_https, host_port)
            }
        };

        // When a proxy is configured:
        // - HTTP  -> connect to proxy, send the full request URI as-is (proxy forwards)
        // - HTTPS -> connect to proxy, issue a CONNECT tunnel, then send request normally
        let (connect_uri, proxy_auth, proxy_tunnel_host) = match &self.proxy {
            Some(proxy) => {
                let auth = proxy.auth_header.clone();
                if need_tunnel {
                    // HTTPS: connect to proxy first, then CONNECT-tunnel to target
                    (proxy.uri.clone(), auth, Some(tunnel_host))
                } else {
                    // HTTP: connect to proxy directly; the connector keeps the original
                    // request unchanged so the full URI goes in the request line.
                    (proxy.uri.clone(), auth, None)
                }
            }
            None => (target_uri, None, None),
        };

        let addr = match &req {
            ConnectRequest::Client(_, _, a) => *a,
            ConnectRequest::Tunnel(_, a) => *a,
        };

        let fut = self.connector.call(ClientConnect {
            uri: connect_uri,
            addr,
        });

        ConnectRequestFuture::Connection {
            fut,
            req: Some(req),
            proxy_auth,
            proxy_tunnel_host,
        }
    }
}

pin_project_lite::pin_project! {
    #[project = ConnectRequestProj]
    pub enum ConnectRequestFuture<Fut, Io>
    where
        Io: ConnectionIo
    {
        Connection {
            #[pin]
            fut: Fut,
            req: Option<ConnectRequest>,
            proxy_auth: Option<HeaderValue>,
            proxy_tunnel_host: Option<String>,
        },
        Tunnel {
            fut: LocalBoxFuture<'static, Result<(ResponseHead, Framed<Connection<Io>, ClientCodec>), SendRequestError>>,
        },
        Client {
            fut: LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>,
        },
        ProxyConnect {
            fut: LocalBoxFuture<'static, Result<Connection<Io>, SendRequestError>>,
            req: Option<ConnectRequest>,
        },
    }
}

impl<Fut, Io> Future for ConnectRequestFuture<Fut, Io>
where
    Fut: Future<Output = Result<Connection<Io>, ConnectError>>,
    Io: ConnectionIo,
{
    type Output = Result<ConnectResponse, SendRequestError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                ConnectRequestProj::Connection {
                    fut,
                    req,
                    proxy_auth,
                    proxy_tunnel_host,
                } => {
                    let connection = ready!(fut.poll(cx))?;
                    let req = req.take().unwrap();

                    if let Some(host) = proxy_tunnel_host.take() {
                        // We connected to the proxy over TCP. Now negotiate a CONNECT
                        // tunnel so subsequent TLS/HTTP2 traffic goes to the real target.
                        let auth_hdr = proxy_auth.take();
                        let fut = Box::pin(proxy_connect_tunnel(connection, host, auth_hdr));
                        self.set(ConnectRequestFuture::ProxyConnect {
                            fut,
                            req: Some(req),
                        });
                        // Loop back to poll the new state immediately.
                        continue;
                    }

                    match req {
                        ConnectRequest::Client(head, body, ..) => {
                            let fut = ConnectRequestFuture::Client {
                                fut: connection.send_request(head, body),
                            };
                            self.set(fut);
                        }
                        ConnectRequest::Tunnel(head, ..) => {
                            let fut = ConnectRequestFuture::Tunnel {
                                fut: connection.open_tunnel(RequestHeadType::from(head)),
                            };
                            self.set(fut);
                        }
                    }
                    // Loop back to poll the newly set state.
                    continue;
                }

                ConnectRequestProj::ProxyConnect { fut, req } => {
                    let connection = ready!(fut.as_mut().poll(cx))?;
                    let req = req.take().unwrap();

                    match req {
                        ConnectRequest::Client(head, body, ..) => {
                            let inner_fut = ConnectRequestFuture::Client {
                                fut: connection.send_request(head, body),
                            };
                            self.set(inner_fut);
                        }
                        ConnectRequest::Tunnel(head, ..) => {
                            let inner_fut = ConnectRequestFuture::Tunnel {
                                fut: connection.open_tunnel(RequestHeadType::from(head)),
                            };
                            self.set(inner_fut);
                        }
                    }
                    continue;
                }

                ConnectRequestProj::Client { fut } => {
                    let (head, payload) = ready!(fut.as_mut().poll(cx))?;
                    return Poll::Ready(Ok(ConnectResponse::Client(ClientResponse::new(
                        head, payload,
                    ))));
                }

                ConnectRequestProj::Tunnel { fut } => {
                    let (head, framed) = ready!(fut.as_mut().poll(cx))?;
                    let framed = framed.into_map_io(|io| Box::new(io) as _);
                    return Poll::Ready(Ok(ConnectResponse::Tunnel(head, framed)));
                }
            }
        }
    }
}

/// Negotiate an HTTP `CONNECT` tunnel with the proxy.
///
/// Writes the `CONNECT host:port HTTP/1.1` preamble to `connection`, reads
/// the proxy's `200 Connection established` response, and returns the
/// connection (now a raw tunnel) or an error.
async fn proxy_connect_tunnel<Io: ConnectionIo>(
    mut connection: Connection<Io>,
    host: String,
    auth: Option<HeaderValue>,
) -> Result<Connection<Io>, SendRequestError> {
    // Build the CONNECT request bytes manually to avoid going through
    // the full HTTP codec machinery (CONNECT is a very simple exchange).
    let mut request = format!("CONNECT {host} HTTP/1.1\r\nHost: {host}\r\n");

    if let Some(auth_hdr) = auth {
        if let Ok(val) = auth_hdr.to_str() {
            request.push_str(&format!("Proxy-Authorization: {val}\r\n"));
        }
    }
    request.push_str("\r\n");

    // `Connection<Io>` derefs to its inner `Io` type which implements
    // `AsyncRead + AsyncWrite` via `ActixStream`/`ConnectionIo`.
    connection
        .write_all(request.as_bytes())
        .await
        .map_err(|err| SendRequestError::Connect(ConnectError::Io(err)))?;

    // Read the proxy response line-by-line until the blank line.
    let mut buf = BytesMut::with_capacity(256);
    let mut raw = [0u8; 256];
    loop {
        let n = connection
            .read(&mut raw)
            .await
            .map_err(|err| SendRequestError::Connect(ConnectError::Io(err)))?;
        if n == 0 {
            return Err(SendRequestError::Connect(ConnectError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "proxy closed connection",
            ))));
        }
        buf.extend_from_slice(&raw[..n]);

        // The proxy response ends with "\r\n\r\n".
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    // Check that the first line is "HTTP/1.x 200 ..."
    let status_line = std::str::from_utf8(&buf)
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");

    if !status_line.contains("200") {
        return Err(SendRequestError::Connect(ConnectError::Io(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("proxy CONNECT failed: {status_line}"),
        ))));
    }

    // Consume the response bytes from the connection's read buffer so they
    // do not leak into the TLS handshake / HTTP request that follows.
    // The bytes are already in `buf`; the connection's stream read them from
    // the socket and they are now consumed.
    drop(buf);

    Ok(connection)
}
