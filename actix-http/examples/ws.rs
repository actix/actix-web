//! Sets up a WebSocket server over TCP and TLS.
//! Sends a heartbeat message every 4 seconds but does not respond to any incoming frames.

extern crate tls_rustls as rustls;

use std::{
    env, io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use actix_codec::Encoder;
use actix_http::{body::BodyStream, error::Error, ws, HttpService, Request, Response};
use actix_rt::time::{interval, Interval};
use actix_server::Server;
use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use futures_core::{ready, Stream};

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "actix=info,h2_ws=info");
    env_logger::init();

    Server::build()
        .bind("tcp", ("127.0.0.1", 8080), || {
            HttpService::build().h1(handler).tcp()
        })?
        .bind("tls", ("127.0.0.1", 8443), || {
            HttpService::build().finish(handler).rustls(tls_config())
        })?
        .run()
        .await
}

async fn handler(req: Request) -> Result<Response<BodyStream<Heartbeat>>, Error> {
    log::info!("handshaking");
    let mut res = ws::handshake(req.head())?;

    // handshake will always fail under HTTP/2

    log::info!("responding");
    Ok(res.message_body(BodyStream::new(Heartbeat::new(ws::Codec::new()))))
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

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        log::trace!("poll");

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

    use rustls::{
        internal::pemfile::{certs, pkcs8_private_keys},
        NoClientAuth, ServerConfig,
    };

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_file = cert.serialize_pem().unwrap();
    let key_file = cert.serialize_private_key_pem();

    let mut config = ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(cert_file.as_bytes());
    let key_file = &mut BufReader::new(key_file.as_bytes());

    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

    config
}
