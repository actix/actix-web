//! simple composite service
//! build: cargo run --example basic --features "ssl"
//! to test: curl https://127.0.0.1:8443/ -k
extern crate actix;
extern crate actix_net;
extern crate futures;
extern crate openssl;
extern crate tokio_io;
extern crate tokio_openssl;
extern crate tokio_tcp;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::{fmt, io};

use futures::{future, Future};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslAcceptorExt;

use actix_net::service::{IntoNewService, NewServiceExt};
use actix_net::Server;

/// Simple logger service, it just prints fact of the new connections
fn logger<T: AsyncRead + AsyncWrite + fmt::Debug>(
    stream: T,
) -> impl Future<Item = T, Error = io::Error> {
    println!("New connection: {:?}", stream);
    future::ok(stream)
}

/// Stateful service, counts number of connections, `ServiceState` is a state
/// for the service
#[derive(Debug)]
struct ServiceState {
    num: Arc<AtomicUsize>,
}

/// Service function for our stateful service
fn service<T: AsyncRead + AsyncWrite>(
    st: &mut ServiceState, _stream: T,
) -> impl Future<Item = (), Error = io::Error> {
    let num = st.num.fetch_add(1, Ordering::Relaxed);
    println!("got ssl connection {:?}", num);
    future::ok(())
}

fn main() {
    let sys = actix::System::new("test");

    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("./examples/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("./examples/cert.pem")
        .unwrap();
    let acceptor = builder.build();

    let num = Arc::new(AtomicUsize::new(0));

    // configure service pipeline
    let srv =
        // service for converting incoming TcpStream to a SslStream<TcpStream>
        (move |stream| {
            SslAcceptorExt::accept_async(&acceptor, stream)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
        })
        // convert closure to a `NewService` and convert init error
        .into_new_service().map_init_err(|_| io::Error::new(io::ErrorKind::Other, ""))

        // .and_then() combinator uses other service to convert incoming `Request` to a `Response`
        // and then uses that response as an input for next service.
        // in this case, on success we use `logger` service
        .and_then(
            // convert function to a Service and related NewService impl
            logger.into_new_service()
                // if service is a function actix-net generate NewService impl with InitError=(),
                // but actix_net::Server requires io::Error as InitError,
                // we need to convert it to a `io::Error`
                .map_init_err(|_| io::Error::new(io::ErrorKind::Other, "")))

        // next service uses two components, service state and service function
        // actix-net generates `NewService` impl that creates `ServiceState` instance for each new service
        // and use `service` function as `Service::call`
        .and_then((service, move || {
            Ok::<_, io::Error>(ServiceState { num: num.clone() })
        }));

    Server::new().bind("0.0.0.0:8443", srv).unwrap().start();

    sys.run();
}
