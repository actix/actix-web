# actix-http

> HTTP primitives for the Actix ecosystem.

[![crates.io](https://img.shields.io/crates/v/actix-http?label=latest)](https://crates.io/crates/actix-http)
[![Documentation](https://docs.rs/actix-http/badge.svg?version=3.0.0-beta.14)](https://docs.rs/actix-http/3.0.0-beta.14)
[![Version](https://img.shields.io/badge/rustc-1.52+-ab6000.svg)](https://blog.rust-lang.org/2021/05/06/Rust-1.52.0.html)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/actix-http.svg)
<br />
[![dependency status](https://deps.rs/crate/actix-http/3.0.0-beta.14/status.svg)](https://deps.rs/crate/actix-http/3.0.0-beta.14)
[![Download](https://img.shields.io/crates/d/actix-http.svg)](https://crates.io/crates/actix-http)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

## Documentation & Resources

- [API Documentation](https://docs.rs/actix-http)
- Minimum Supported Rust Version (MSRV): 1.52

## Example

```rust
use actix_http::HttpService;
use actix_server::Server;
use actix_service::map_config;
use actix_web::{dev::AppConfig, get, App};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    Server::build()
        .bind("hello-world", "127.0.0.1:8080", || {
            // construct actix-web app.
            let app = App::new().service(index);
            HttpService::build()
                // pass the app to service builder.
                // map_config is used to map App's configuration to ServiceBuilder.
                .finish(map_config(app, |_| AppConfig::default()))
                .tcp()
        })?
        .run()
        .await
}

#[get("/")]
async fn index() -> &'static str {
    "Hello,World from actix-web!"
}
```

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-http crate is organized under the terms of the
Contributor Covenant, the maintainer of actix-http, @fafhrd91, promises to
intervene to uphold that code of conduct.
