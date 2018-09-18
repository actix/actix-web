//! simple composite service
//! build: cargo run --example basic --features "ssl"
//! to test: curl https://127.0.0.1:8443/ -k
extern crate actix;
extern crate actix_net;
extern crate env_logger;
extern crate futures;
extern crate openssl;
extern crate tokio_io;
extern crate tokio_openssl;
extern crate tokio_tcp;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::{env, fmt};

use futures::{future, Future};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslAcceptorExt;

use actix_net::server::Server;
use actix_net::service::{IntoNewService, NewServiceExt};

/// Simple logger service, it just prints fact of the new connections
fn logger<T: AsyncRead + AsyncWrite + fmt::Debug>(
    stream: T,
) -> impl Future<Item = T, Error = ()> {
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
) -> impl Future<Item = (), Error = ()> {
    let num = st.num.fetch_add(1, Ordering::Relaxed);
    println!("got ssl connection {:?}", num);
    future::ok(())
}

fn main() {
    env::set_var("RUST_LOG", "actix_net=trace");
    env_logger::init();

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

    // bind socket address and start workers. By default server uses number of
    // available logical cpu as threads count. actix net start separate
    // instances of service pipeline in each worker.
    Server::default()
        .bind(
            // configure service pipeline
            "basic",
            "0.0.0.0:8443",
            move || {
                let num = num.clone();
                let acceptor = acceptor.clone();

                // service for converting incoming TcpStream to a SslStream<TcpStream>
                (move |stream| {
                    SslAcceptorExt::accept_async(&acceptor, stream)
                        .map_err(|e| println!("Openssl error: {}", e))
                })
                // convert closure to a `NewService`
                .into_new_service()
                // .and_then() combinator uses other service to convert incoming `Request` to a
                // `Response` and then uses that response as an input for next
                // service. in this case, on success we use `logger` service
                .and_then(logger)
                // next service uses two components, service state and service function
                // actix-net generates `NewService` impl that creates `ServiceState` instance
                // for each new service and use `service` function as
                // `Service::call`
                .and_then((service, move || Ok(ServiceState { num: num.clone() })))
            },
        ).unwrap()
        .start();

    sys.run();
}
