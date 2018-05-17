## Migration from 0.5 to 0.6

* `Path<T>` extractor return `ErrorNotFound` on failure instead of `ErrorBadRequest`

* `ws::Message::Close` now includes optional close reason.
  `ws::CloseCode::Status` and `ws::CloseCode::Empty` have been removed.

* `HttpServer::threads()` renamed to `HttpServer::workers()`.

* `HttpServer::start_ssl()` and `HttpServer::start_tls()` deprecated.
  Use `HttpServer::bind_ssl()` and `HttpServer::bind_tls()` instead.

* `HttpRequest::extensions()` returns read only reference to the request's Extension
  `HttpRequest::extensions_mut()` returns mutable reference.

* Instead of 

   `use actix_web::middleware::{
        CookieSessionBackend, CookieSessionError, RequestSession,
        Session, SessionBackend, SessionImpl, SessionStorage};`
                                
  use `actix_web::middleware::session`

   `use actix_web::middleware::session{CookieSessionBackend, CookieSessionError,
        RequestSession, Session, SessionBackend, SessionImpl, SessionStorage};`

* `FromRequest::from_request()` accepts mutable reference to a request

* `FromRequest::Result` has to implement `Into<Reply<Self>>`

* [`Responder::respond_to()`](
  https://actix.rs/actix-web/actix_web/trait.Responder.html#tymethod.respond_to)
  is generic over `S`

*  Use `Query` extractor instead of HttpRequest::query()`.

   ```rust
   fn index(q: Query<HashMap<String, String>>) -> Result<..> {
       ...
   }
   ```

   or

   ```rust
   let q = Query::<HashMap<String, String>>::extract(req);
   ```

* Websocket operations are implemented as `WsWriter` trait.
  you need to use `use actix_web::ws::WsWriter`


## Migration from 0.4 to 0.5

* `HttpResponseBuilder::body()`, `.finish()`, `.json()`
   methods return `HttpResponse` instead of `Result<HttpResponse>`

* `actix_web::Method`, `actix_web::StatusCode`, `actix_web::Version`
   moved to `actix_web::http` module

* `actix_web::header` moved to `actix_web::http::header`

* `NormalizePath` moved to `actix_web::http` module

* `HttpServer` moved to `actix_web::server`, added new `actix_web::server::new()` function,
  shortcut for `actix_web::server::HttpServer::new()`

* `DefaultHeaders` middleware does not use separate builder, all builder methods moved to type itself

* `StaticFiles::new()`'s show_index parameter removed, use `show_files_listing()` method instead.

* `CookieSessionBackendBuilder` removed, all methods moved to `CookieSessionBackend` type

* `actix_web::httpcodes` module is deprecated, `HttpResponse::Ok()`, `HttpResponse::Found()` and other `HttpResponse::XXX()`
   functions should be used instead

* `ClientRequestBuilder::body()` returns `Result<_, actix_web::Error>`
  instead of `Result<_, http::Error>`

* `Application` renamed to a `App`

* `actix_web::Reply`, `actix_web::Resource` moved to `actix_web::dev`
