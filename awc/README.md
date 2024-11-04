# `awc` (Actix Web Client)

> Async HTTP and WebSocket client library.

<!-- prettier-ignore-start -->

[![crates.io](https://img.shields.io/crates/v/awc?label=latest)](https://crates.io/crates/awc)
[![Documentation](https://docs.rs/awc/badge.svg?version=3.5.1)](https://docs.rs/awc/3.5.1)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/awc)
[![Dependency Status](https://deps.rs/crate/awc/3.5.1/status.svg)](https://deps.rs/crate/awc/3.5.1)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

<!-- prettier-ignore-end -->

## Examples

[Example project using TLS-enabled client →](https://github.com/actix/examples/tree/master/https-tls/awc-https)

Basic usage:

```rust
use actix_rt::System;
use awc::Client;

fn main() {
    System::new().block_on(async {
        let client = Client::default();

        let res = client
            .get("http://www.rust-lang.org")    // <- Create request builder
            .insert_header(("User-Agent", "Actix-web"))
            .send()                             // <- Send http request
            .await;

        println!("Response: {:?}", res);        // <- server http response
    });
}
```
