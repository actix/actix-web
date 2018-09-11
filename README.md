# Actix net [![Build Status](https://travis-ci.org/actix/actix-net.svg?branch=master)](https://travis-ci.org/actix/actix-net) [![codecov](https://codecov.io/gh/actix/actix-net/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-net) [![crates.io](https://meritbadge.herokuapp.com/actix-net)](https://crates.io/crates/actix-net) [![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

Actix net - framework for compossible network services

## Documentation & community resources

* [API Documentation (Development)](https://actix.rs/actix-net/actix_net/)
* [Chat on gitter](https://gitter.im/actix/actix)
* Cargo package: [actix-net](https://crates.io/crates/actix-net)
* Minimum supported Rust version: 1.26 or later

## Example

```rust
fn main() {
    let sys = actix::System::new("test");

    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_private_key_file("./examples/key.pem", SslFiletype::PEM).unwrap();
    builder.set_certificate_chain_file("./examples/cert.pem").unwrap();
    let acceptor = builder.build();

    let num = Arc::new(AtomicUsize::new(0));

    // bind socket address and start workers. By default server uses number of
    // available logical cpu as threads count. actix net start separate
    // instances of service pipeline in each worker.
    Server::default()
        .bind(
            // configure service pipeline
            "0.0.0.0:8443",
            move || {
                let num = num.clone();
                let acceptor = acceptor.clone();

                // service for converting incoming TcpStream to a SslStream<TcpStream>
                (move |stream| {
                SslAcceptorExt::accept_async(&acceptor, stream)
                    .map_err(|e| println!("Openssl error: {}", e))
            })
            // convert closure to a `NewService`
            .into_new_service()

            // .and_then() combinator uses other service to convert incoming `Request` to a `Response`
            // and then uses that response as an input for next service.
            // in this case, on success we use `logger` service
            .and_then(logger)

            // next service uses two components, service state and service function
            // actix-net generates `NewService` impl that creates `ServiceState` instance for each new service
            // and use `service` function as `Service::call`
            .and_then((service, move || {
                Ok(ServiceState { num: num.clone() })
            }))
            },
        ).unwrap()
        .start();

    sys.run();
}
```

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-net crate is organized under the terms of the
Contributor Covenant, the maintainer of actix-net, @fafhrd91, promises to
intervene to uphold that code of conduct.
