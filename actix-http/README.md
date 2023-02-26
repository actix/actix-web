# actix-http

> HTTP primitives for the Actix ecosystem.

[![crates.io](https://img.shields.io/crates/v/actix-http?label=latest)](https://crates.io/crates/actix-http)
[![Documentation](https://docs.rs/actix-http/badge.svg?version=3.3.0)](https://docs.rs/actix-http/3.3.0)
![Version](https://img.shields.io/badge/rustc-1.59+-ab6000.svg)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/actix-http.svg)
<br />
[![dependency status](https://deps.rs/crate/actix-http/3.3.0/status.svg)](https://deps.rs/crate/actix-http/3.3.0)
[![Download](https://img.shields.io/crates/d/actix-http.svg)](https://crates.io/crates/actix-http)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

## Documentation & Resources

- [API Documentation](https://docs.rs/actix-http)
- Minimum Supported Rust Version (MSRV): 1.54

## Example

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

## License

This project is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
- MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-http crate is organized under the terms of the
Contributor Covenant, the maintainer of actix-http, @fafhrd91, promises to
intervene to uphold that code of conduct.
