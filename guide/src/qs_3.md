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
   let app = Application::default("/prefix")
       .resource("/index.html", |r| r.handler(Method::GET, index)
       .finish()
```

In this example application with `/prefix` prefix and `index.html` resource
get created. This resource is available as on `/prefix/index.html` url.
