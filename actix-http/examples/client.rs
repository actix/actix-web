use actix_http::{client, Error};
use actix_rt::System;
use bytes::BytesMut;
use futures::{future::lazy, Future, Stream};

fn main() -> Result<(), Error> {
    std::env::set_var("RUST_LOG", "actix_http=trace");
    env_logger::init();

    System::new("test").block_on(lazy(|| {
        let mut connector = client::Connector::new().service();

        client::ClientRequest::get("https://www.rust-lang.org/") // <- Create request builder
            .header("User-Agent", "Actix-web")
            .finish()
            .unwrap()
            .send(&mut connector) // <- Send http request
            .from_err()
            .and_then(|response| {
                // <- server http response
                println!("Response: {:?}", response);

                // read response body
                response
                    .from_err()
                    .fold(BytesMut::new(), move |mut acc, chunk| {
                        acc.extend_from_slice(&chunk);
                        Ok::<_, Error>(acc)
                    })
                    .map(|body| println!("Downloaded: {:?} bytes", body.len()))
            })
    }))
}
