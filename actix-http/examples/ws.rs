//! Sets up a WebSocket server over TCP and TLS.
//! Sends a heartbeat message every 4 seconds but does not respond to any incoming frames.

extern crate tls_rustls_023 as rustls;

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use actix_http::{body::BodyStream, error::Error, ws, HttpService, Request, Response};
use actix_rt::time::{interval, Interval};
use actix_server::Server;
use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use futures_core::{ready, Stream};
use tokio_util::codec::Encoder;
use tracing::{info, trace};

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("tcp", ("127.0.0.1", 8080), || {
            HttpService::build().h1(handler).tcp()
        })?
        .bind("tls", ("127.0.0.1", 8443), || {
            HttpService::build()
                .finish(handler)
                .rustls_0_23(tls_config())
        })?
        .run()
        .await
}

async fn handler(req: Request) -> Result<Response<BodyStream<Heartbeat>>, Error> {
    info!("handshaking");
    let mut res = ws::handshake(req.head())?;

    // handshake will always fail under HTTP/2

    info!("responding");
    res.message_body(BodyStream::new(Heartbeat::new(ws::Codec::new())))
}

struct Heartbeat {
    codec: ws::Codec,
    interval: Interval,
}

impl Heartbeat {
    fn new(codec: ws::Codec) -> Self {
        Self {
            codec,
            interval: interval(Duration::from_secs(4)),
        }
    }
}

impl Stream for Heartbeat {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        trace!("poll");

        ready!(self.as_mut().interval.poll_tick(cx));

        let mut buffer = BytesMut::new();

        self.as_mut()
            .codec
            .encode(
                ws::Message::Text(ByteString::from_static("hello world")),
                &mut buffer,
            )
            .unwrap();

        Poll::Ready(Some(Ok(buffer.freeze())))
    }
}

fn tls_config() -> rustls::ServerConfig {
    use std::io::BufReader;

    use rustls_pemfile::{certs, pkcs8_private_keys};

    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
    let cert_file = cert.pem();
    let key_file = key_pair.serialize_pem();

    let cert_file = &mut BufReader::new(cert_file.as_bytes());
    let key_file = &mut BufReader::new(key_file.as_bytes());

    let cert_chain = certs(cert_file).collect::<Result<Vec<_>, _>>().unwrap();
    let mut keys = pkcs8_private_keys(key_file)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            cert_chain,
            rustls::pki_types::PrivateKeyDer::Pkcs8(keys.remove(0)),
        )
        .unwrap();

    config.alpn_protocols.push(b"http/1.1".to_vec());
    config.alpn_protocols.push(b"h2".to_vec());

    config
}
