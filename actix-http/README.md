# Actix http [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web)  [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](https://meritbadge.herokuapp.com/actix-http)](https://crates.io/crates/actix-http) [![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

Actix http

## Documentation & community resources

* [User Guide](https://actix.rs/docs/)
* [API Documentation](https://docs.rs/actix-http/)
* [Chat on gitter](https://gitter.im/actix/actix)
* Cargo package: [actix-http](https://crates.io/crates/actix-http)
* Minimum supported Rust version: 1.31 or later

## Example

```rust
// see examples/framed_hello.rs for complete list of used crates.
extern crate actix_http;
use actix_http::{h1, Response, ServiceConfig};

fn main() {
    Server::new().bind("framed_hello", "127.0.0.1:8080", || {
        IntoFramed::new(|| h1::Codec::new(ServiceConfig::default()))	// <- create h1 codec
            .and_then(TakeItem::new().map_err(|_| ()))	                // <- read one request
            .and_then(|(_req, _framed): (_, Framed<_, _>)| {	        // <- send response and close conn
                SendResponse::send(_framed, Response::Ok().body("Hello world!"))
                    .map_err(|_| ())
                    .map(|_| ())
            })
    }).unwrap().run();
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
