# Application

Actix web provides some primitives to build web servers and applications with Rust.
It provides routing, middlewares, pre-processing of requests, and post-processing of responses,
websocket protcol handling, multipart streams, etc.

All actix web server is built around `Application` instance.
It is used for registering handlers for routes and resources, middlewares.
Also it stores applicationspecific state that is shared accross all handlers 
within same application.

Application acts as namespace for all routes, i.e all routes for specific application
has same url path prefix:

```rust,ignore
# extern crate actix_web;
# extern crate tokio_core;
# use actix_web::*;
# fn index(req: HttpRequest) -> &'static str {
#    "Hello world!"
# }
# fn main() {
   let app = Application::new("/prefix")
       .resource("/index.html", |r| r.method(Method::GET).f(index))
       .finish()
# }
```

In this example application with `/prefix` prefix and `index.html` resource
get created. This resource is available as on `/prefix/index.html` url.

Multiple applications could be served with one server:

```rust
# extern crate actix_web;
# extern crate tokio_core;
use std::net::SocketAddr;
use actix_web::*;
use tokio_core::net::TcpStream;

fn main() {
    HttpServer::<TcpStream, SocketAddr, _>::new(vec![
        Application::new("/app1")
            .resource("/", |r| r.f(|r| httpcodes::HTTPOk)),
        Application::new("/app2")
            .resource("/", |r| r.f(|r| httpcodes::HTTPOk)),
        Application::new("/")
            .resource("/", |r| r.f(|r| httpcodes::HTTPOk)),
    ]);
}
```

All `/app1` requests route to first application, `/app2` to second and then all other to third.
