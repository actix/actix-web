# actix-http

> HTTP primitives for the Actix ecosystem.

[![crates.io](https://img.shields.io/crates/v/actix-http?label=latest)](https://crates.io/crates/actix-http)
[![Documentation](https://docs.rs/actix-http/badge.svg?version=2.2.0)](https://docs.rs/actix-http/2.2.0)
![Apache 2.0 or MIT licensed](https://img.shields.io/crates/l/actix-http)
[![Dependency Status](https://deps.rs/crate/actix-http/2.2.0/status.svg)](https://deps.rs/crate/actix-http/2.2.0)
[![Join the chat at https://gitter.im/actix/actix-web](https://badges.gitter.im/actix/actix-web.svg)](https://gitter.im/actix/actix-web?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

## Documentation & Resources

- [API Documentation](https://docs.rs/actix-http)
- [Chat on Gitter](https://gitter.im/actix/actix-web)
- Minimum Supported Rust Version (MSRV): 1.42.0

## Example

```rust
use std::{env, io};

use actix_http::{HttpService, Response};
use actix_server::Server;
use futures_util::future;
use http::header::HeaderValue;
use log::info;

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

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-http crate is organized under the terms of the
Contributor Covenant, the maintainer of actix-http, @fafhrd91, promises to
intervene to uphold that code of conduct.
