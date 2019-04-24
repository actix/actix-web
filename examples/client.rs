use actix_http::Error;
use actix_rt::System;
use futures::{future::lazy, Future};

fn main() -> Result<(), Error> {
    std::env::set_var("RUST_LOG", "actix_http=trace");
    env_logger::init();

    System::new("test").block_on(lazy(|| {
        awc::Client::new()
            .get("https://www.rust-lang.org/") // <- Create request builder
            .header("User-Agent", "Actix-web")
            .send() // <- Send http request
            .from_err()
            .and_then(|mut response| {
                // <- server http response
                println!("Response: {:?}", response);

                // read response body
                response
                    .body()
                    .from_err()
                    .map(|body| println!("Downloaded: {:?} bytes", body.len()))
            })
    }))
}
