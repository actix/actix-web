# Actix http client [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](https://meritbadge.herokuapp.com/awc)](https://crates.io/crates/awc) [![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

An HTTP Client

## Documentation & community resources

* [User Guide](https://actix.rs/docs/)
* [API Documentation](https://docs.rs/awc/)
* [Chat on gitter](https://gitter.im/actix/actix)
* Cargo package: [awc](https://crates.io/crates/awc)
* Minimum supported Rust version: 1.33 or later

## Example

```rust
use actix_rt::System;
use awc::Client;
use futures::future::{Future, lazy};

fn main() {
    System::new("test").block_on(lazy(|| {
       let mut client = Client::default();

       client.get("http://www.rust-lang.org") // <- Create request builder
          .header("User-Agent", "Actix-web")
          .send()                             // <- Send http request
          .and_then(|response| {              // <- server http response
               println!("Response: {:?}", response);
               Ok(())
          })
    }));
}
```
