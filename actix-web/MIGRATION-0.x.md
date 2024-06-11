# 0.7.15

- The `' '` character is not percent decoded anymore before matching routes. If you need to use it in your routes, you should use `%20`.

instead of

```rust
fn main() {
      let app = App::new().resource("/my index", |r| {
          r.method(http::Method::GET)
                .with(index);
      });
}
```

use

```rust
fn main() {
      let app = App::new().resource("/my%20index", |r| {
          r.method(http::Method::GET)
                .with(index);
      });
}
```

- If you used `AsyncResult::async` you need to replace it with `AsyncResult::future`

# 0.7.4

- `Route::with_config()`/`Route::with_async_config()` always passes configuration objects as tuple even for handler with one parameter.

# 0.7

- `HttpRequest` does not implement `Stream` anymore. If you need to read request payload use `HttpMessage::payload()` method.

instead of

```rust
fn index(req: HttpRequest) -> impl Responder {
      req
        .from_err()
        .fold(...)
        ....
}
```

use `.payload()`

```rust
fn index(req: HttpRequest) -> impl Responder {
      req
        .payload()  // <- get request payload stream
        .from_err()
        .fold(...)
        ....
}
```

- [Middleware](https://actix.rs/actix-web/actix_web/middleware/trait.Middleware.html) trait uses `&HttpRequest` instead of `&mut HttpRequest`.

- Removed `Route::with2()` and `Route::with3()` use tuple of extractors instead.

instead of

```rust
fn index(query: Query<..>, info: Json<MyStruct) -> impl Responder {}
```

use tuple of extractors and use `.with()` for registration:

```rust
fn index((query, json): (Query<..>, Json<MyStruct)) -> impl Responder {}
```

- `Handler::handle()` uses `&self` instead of `&mut self`

- `Handler::handle()` accepts reference to `HttpRequest<_>` instead of value

- Removed deprecated `HttpServer::threads()`, use [HttpServer::workers()](https://actix.rs/actix-web/actix_web/server/struct.HttpServer.html#method.workers) instead.

- Renamed `client::ClientConnectorError::Connector` to `client::ClientConnectorError::Resolver`

- `Route::with()` does not return `ExtractorConfig`, to configure extractor use `Route::with_config()`

instead of

```rust
fn main() {
      let app = App::new().resource("/index.html", |r| {
          r.method(http::Method::GET)
                .with(index)
                .limit(4096);  // <- limit size of the payload
      });
}
```

use

```rust

fn main() {
      let app = App::new().resource("/index.html", |r| {
          r.method(http::Method::GET)
                .with_config(index, |cfg| { // <- register handler
                    cfg.limit(4096);  // <- limit size of the payload
                  })
      });
}
```

- `Route::with_async()` does not return `ExtractorConfig`, to configure extractor use `Route::with_async_config()`

# 0.6

- `Path<T>` extractor return `ErrorNotFound` on failure instead of `ErrorBadRequest`

- `ws::Message::Close` now includes optional close reason. `ws::CloseCode::Status` and `ws::CloseCode::Empty` have been removed.

- `HttpServer::threads()` renamed to `HttpServer::workers()`.

- `HttpServer::start_ssl()` and `HttpServer::start_tls()` deprecated. Use `HttpServer::bind_ssl()` and `HttpServer::bind_tls()` instead.

- `HttpRequest::extensions()` returns read only reference to the request's Extension `HttpRequest::extensions_mut()` returns mutable reference.

- Instead of

  `use actix_web::middleware::{ CookieSessionBackend, CookieSessionError, RequestSession, Session, SessionBackend, SessionImpl, SessionStorage};`

  use `actix_web::middleware::session`

  `use actix_web::middleware::session{CookieSessionBackend, CookieSessionError, RequestSession, Session, SessionBackend, SessionImpl, SessionStorage};`

- `FromRequest::from_request()` accepts mutable reference to a request

- `FromRequest::Result` has to implement `Into<Reply<Self>>`

- [`Responder::respond_to()`](https://actix.rs/actix-web/actix_web/trait.Responder.html#tymethod.respond_to) is generic over `S`

- Use `Query` extractor instead of HttpRequest::query()`.

```rust
fn index(q: Query<HashMap<String, String>>) -> Result<..> {
    ...
}
```

or

```rust
let q = Query::<HashMap<String, String>>::extract(req);
```

- Websocket operations are implemented as `WsWriter` trait. you need to use `use actix_web::ws::WsWriter`

# 0.5

- `HttpResponseBuilder::body()`, `.finish()`, `.json()` methods return `HttpResponse` instead of `Result<HttpResponse>`

- `actix_web::Method`, `actix_web::StatusCode`, `actix_web::Version` moved to `actix_web::http` module

- `actix_web::header` moved to `actix_web::http::header`

- `NormalizePath` moved to `actix_web::http` module

- `HttpServer` moved to `actix_web::server`, added new `actix_web::server::new()` function, shortcut for `actix_web::server::HttpServer::new()`

- `DefaultHeaders` middleware does not use separate builder, all builder methods moved to type itself

- `StaticFiles::new()`'s show_index parameter removed, use `show_files_listing()` method instead.

- `CookieSessionBackendBuilder` removed, all methods moved to `CookieSessionBackend` type

- `actix_web::httpcodes` module is deprecated, `HttpResponse::Ok()`, `HttpResponse::Found()` and other `HttpResponse::XXX()` functions should be used instead

- `ClientRequestBuilder::body()` returns `Result<_, actix_web::Error>` instead of `Result<_, http::Error>`

- `Application` renamed to a `App`

- `actix_web::Reply`, `actix_web::Resource` moved to `actix_web::dev`
