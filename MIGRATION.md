## Migration from 0.5 to 0.6

* `ws::Message::Close` now includes optional close reason.
  `ws::CloseCode::Status` and `ws::CloseCode::Empty` have been removed.

* `HttpServer::start_ssl()` and `HttpServer::start_tls()` deprecated.
  Use `HttpServer::bind_ssl()` and `HttpServer::bind_tls()` instead.


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
