# `actix-http`

> HTTP types and services for the Actix ecosystem.

<!-- prettier-ignore-start -->

[![crates.io](https://img.shields.io/crates/v/actix-http?label=latest)](https://crates.io/crates/actix-http)
[![Documentation](https://docs.rs/actix-http/badge.svg?version=3.18.13)](https://docs.rs/actix-http/3.18.13)
![Version](https://img.shields.io/badge/rustc-1.88+-ab6000.svg)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/actix-http.svg)
<br />
[![dependency status](https://deps.rs/crate/actix-http/3.18.13/status.svg)](https://deps.rs/crate/actix-http/3.18.13)
[![Download](https://img.shields.io/crates/d/actix-http.svg)](https://crates.io/crates/actix-http)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

<!-- prettier-ignore-end -->

## Examples

```rust
use std::{env, io};

use actix_http::{HttpService, Response};
use actix_server::Server;
use futures_util::future;
use http::header::HeaderValue;
use tracing::info;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "hello_world=info");
    env_logger::init();

    Server::build()
        .bind("hello-world", "127.0.0.1:8080", || {
            HttpService::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .finish(|_req| {
                    info!("{:?}", _req);
                    let mut res = Response::Ok();
                    res.header("x-head", HeaderValue::from_static("dummy value!"));
                    future::ok::<_, ()>(res.body("Hello world!"))
                })
                .tcp()
        })?
        .run()
        .await
}
```

## Sponsors

<a href="https://www.blacksmith.sh/">
  <img src="https://raw.githubusercontent.com/actix/actix-web/main/.github/sponsors/blacksmith.png" alt="CI powered by Blacksmith" width="280" />
</a>
