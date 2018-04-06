# Server

The [**HttpServer**](../actix_web/server/struct.HttpServer.html) type is responsible for
serving http requests.

`HttpServer` accepts an application factory as a parameter, and the
application factory must have `Send` + `Sync` boundaries. More about that in the
*multi-threading* section.

To bind to a specific socket address, `bind()` must be used, and it may be called multiple times.
To start the http server, one of the start methods.

- use `start()` for a simple server
- use `start_tls()` or `start_ssl()` for a ssl server

`HttpServer` is an actix actor. It must be initialized within a properly configured actix system:

```rust
# extern crate actix;
# extern crate actix_web;
use actix_web::{server::HttpServer, App, HttpResponse};

fn main() {
    let sys = actix::System::new("guide");

    HttpServer::new(
        || App::new()
            .resource("/", |r| r.f(|_| HttpResponse::Ok())))
        .bind("127.0.0.1:59080").unwrap()
        .start();

#     actix::Arbiter::system().do_send(actix::msgs::SystemExit(0));
    let _ = sys.run();
}
```

> It is possible to start a server in a separate thread with the `spawn()` method. In that
> case the server spawns a new thread and creates a new actix system in it. To stop
> this server, send a `StopServer` message.

`HttpServer` is implemented as an actix actor. It is possible to communicate with the server
via a messaging system. All start methods, e.g. `start()` and `start_ssl()`, return the
address of the started http server. It accepts several messages:

- `PauseServer` - Pause accepting incoming connections
- `ResumeServer` - Resume accepting incoming connections
- `StopServer` - Stop incoming connection processing, stop all workers and exit

```rust
# extern crate futures;
# extern crate actix;
# extern crate actix_web;
# use futures::Future;
use std::thread;
use std::sync::mpsc;
use actix_web::{server, App, HttpResponse};

fn main() {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = actix::System::new("http-server");
        let addr = server::new(
            || App::new()
                .resource("/", |r| r.f(|_| HttpResponse::Ok())))
            .bind("127.0.0.1:0").expect("Can not bind to 127.0.0.1:0")
            .shutdown_timeout(60)    // <- Set shutdown timeout to 60 seconds
            .start();
        let _ = tx.send(addr);
        let _ = sys.run();
    });

    let addr = rx.recv().unwrap();
    let _ = addr.send(
         server::StopServer{graceful:true}).wait(); // <- Send `StopServer` message to server.
}
```

## Multi-threading

`HttpServer` automatically starts an number of http workers, by default
this number is equal to number of logical CPUs in the system. This number
can be overridden with the `HttpServer::threads()` method.

```rust
# extern crate actix_web;
# extern crate tokio_core;
use actix_web::{App, HttpResponse, server::HttpServer};

fn main() {
    HttpServer::new(
        || App::new()
            .resource("/", |r| r.f(|_| HttpResponse::Ok())))
        .threads(4); // <- Start 4 workers
}
```

The server creates a separate application instance for each created worker. Application state
is not shared between threads. To share state, `Arc` could be used.

> Application state does not need to be `Send` and `Sync`,
> but factories must be `Send` + `Sync`.

## SSL

There are two features for ssl server: `tls` and `alpn`. The `tls` feature is for `native-tls`
integration and `alpn` is for `openssl`.

```toml
[dependencies]
actix-web = { git = "https://github.com/actix/actix-web", features=["alpn"] }
```

```rust,ignore
use std::fs::File;
use actix_web::*;

fn main() {
    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_private_key_file("key.pem", SslFiletype::PEM).unwrap();
    builder.set_certificate_chain_file("cert.pem").unwrap();

    server::new(
        || App::new()
            .resource("/index.html", |r| r.f(index)))
        .bind("127.0.0.1:8080").unwrap()
        .serve_ssl(builder).unwrap();
}
```

> **Note**: the *HTTP/2.0* protocol requires
> [tls alpn](https://tools.ietf.org/html/rfc7301).
> At the moment, only `openssl` has `alpn` support.
> For a full example, check out
> [examples/tls](https://github.com/actix/actix-web/tree/master/examples/tls).

## Keep-Alive

Actix can wait for requests on a keep-alive connection.

> *keep alive* connection behavior is defined by server settings.

- `75`, `Some(75)`, `KeepAlive::Timeout(75)` - enable 75 second *keep alive* timer.
- `None` or `KeepAlive::Disabled` - disable *keep alive*.
- `KeepAlive::Tcp(75)` - use `SO_KEEPALIVE` socket option.

```rust
# extern crate actix_web;
# extern crate tokio_core;
use actix_web::{server, App, HttpResponse};

fn main() {
    server::new(||
        App::new()
            .resource("/", |r| r.f(|_| HttpResponse::Ok())))
        .keep_alive(75); // <- Set keep-alive to 75 seconds

    server::new(||
        App::new()
            .resource("/", |r| r.f(|_| HttpResponse::Ok())))
        .keep_alive(server::KeepAlive::Tcp(75)); // <- Use `SO_KEEPALIVE` socket option.

    server::new(||
        App::new()
            .resource("/", |r| r.f(|_| HttpResponse::Ok())))
        .keep_alive(None); // <- Disable keep-alive
}
```

If the first option is selected, then *keep alive* state is
calculated based on the response's *connection-type*. By default
`HttpResponse::connection_type` is not defined. In that case *keep alive* is
defined by the request's http version.

> *keep alive* is **off** for *HTTP/1.0* and is **on** for *HTTP/1.1* and *HTTP/2.0*.

*Connection type* can be change with `HttpResponseBuilder::connection_type()` method.

```rust
# extern crate actix_web;
use actix_web::{HttpRequest, HttpResponse, http};

fn index(req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .connection_type(http::ConnectionType::Close) // <- Close connection
        .force_close()                                // <- Alternative method
        .finish()
}
# fn main() {}
```

## Graceful shutdown

`HttpServer` supports graceful shutdown. After receiving a stop signal, workers
have a specific amount of time to finish serving requests. Any workers still alive after the
timeout are force-dropped. By default the shutdown timeout is set to 30 seconds.
You can change this parameter with the `HttpServer::shutdown_timeout()` method.

You can send a stop message to the server with the server address and specify if you want
graceful shutdown or not. The `start()` methods returns address of the server.

`HttpServer` handles several OS signals. *CTRL-C* is available on all OSs,
other signals are available on unix systems.

- *SIGINT* - Force shutdown workers
- *SIGTERM* - Graceful shutdown workers
- *SIGQUIT* - Force shutdown workers

> It is possible to disable signal handling with `HttpServer::disable_signals()` method.
