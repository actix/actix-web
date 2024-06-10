# `actix-test`

<!-- prettier-ignore-start -->

[![crates.io](https://img.shields.io/crates/v/actix-test?label=latest)](https://crates.io/crates/actix-test)
[![Documentation](https://docs.rs/actix-test/badge.svg?version=0.1.5)](https://docs.rs/actix-test/0.1.5)
![Version](https://img.shields.io/badge/rustc-1.72+-ab6000.svg)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/actix-test.svg)
<br />
[![dependency status](https://deps.rs/crate/actix-test/0.1.5/status.svg)](https://deps.rs/crate/actix-test/0.1.5)
[![Download](https://img.shields.io/crates/d/actix-test.svg)](https://crates.io/crates/actix-test)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

<!-- prettier-ignore-end -->

<!-- cargo-rdme start -->

Integration testing tools for Actix Web applications.

The main integration testing tool is [`TestServer`]. It spawns a real HTTP server on an unused port and provides methods that use a real HTTP client. Therefore, it is much closer to real-world cases than using `init_service`, which skips HTTP encoding and decoding.

## Examples

```rust
use actix_web::{get, web, test, App, HttpResponse, Error, Responder};

#[get("/")]
async fn my_handler() -> Result<impl Responder, Error> {
    Ok(HttpResponse::Ok())
}

#[actix_rt::test]
async fn test_example() {
    let srv = actix_test::start(||
        App::new().service(my_handler)
    );

    let req = srv.get("/");
    let res = req.send().await.unwrap();

    assert!(res.status().is_success());
}
```

<!-- cargo-rdme end -->
