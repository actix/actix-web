extern crate actix;
extern crate actix_net;
extern crate futures;
extern crate openssl;
extern crate tokio_io;
extern crate tokio_tcp;

use std::io;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use futures::{future, Future};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use tokio_io::{AsyncRead, AsyncWrite};

use actix_net::service::NewServiceExt;
use actix_net::{ssl, Server};

#[derive(Debug)]
struct ServiceState {
    num: Arc<AtomicUsize>,
}

fn service<T: AsyncRead + AsyncWrite>(
    st: &mut ServiceState, stream: T,
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

    let num = Arc::new(AtomicUsize::new(0));

    // configure service
    let srv = ssl::OpensslService::new(builder).and_then((service, move || {
        Ok::<_, io::Error>(ServiceState { num: num.clone() })
    }));

    Server::new().bind("0.0.0.0:8443", srv).unwrap().start();

    sys.run();
}
