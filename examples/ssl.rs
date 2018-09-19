extern crate actix;
extern crate actix_net;
extern crate futures;
extern crate openssl;
extern crate tokio_io;
extern crate tokio_tcp;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use futures::{future, Future};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use tokio_io::{AsyncRead, AsyncWrite};

use actix_net::server::Server;
use actix_net::service::NewServiceExt;
use actix_net::ssl;

#[derive(Debug)]
struct ServiceState {
    num: Arc<AtomicUsize>,
}

fn service<T: AsyncRead + AsyncWrite>(
    st: &mut ServiceState, _: T,
) -> impl Future<Item = (), Error = ()> {
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

    let num = Arc::new(AtomicUsize::new(0));
    let openssl = ssl::OpensslAcceptor::new(builder.build());

    // server start mutiple workers, it runs supplied `Fn` in each worker.
    Server::default()
        .bind("test-ssl", "0.0.0.0:8443", move || {
            let num = num.clone();

            // configure service
            openssl
                .clone()
                .map_err(|e| println!("Openssl error: {}", e))
                .and_then(move |_| {
                    let num = num.fetch_add(1, Ordering::Relaxed);
                    println!("got ssl connection {:?}", num);
                    future::ok(())
                })
        }).unwrap()
        .start();

    sys.run();
}
