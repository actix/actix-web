# Changes

## [0.7.4] - 2018-08-xx

### Added

* Added `HttpServer::maxconn()` and `HttpServer::maxconnrate()`,
  accept backpressure #250

* Allow to customize connection handshake process via `HttpServer::listen_with()`
  and `HttpServer::bind_with()` methods

### Changed

* native-tls - 0.2

### Fixed

* Use zlib instead of raw deflate for decoding and encoding payloads with
  `Content-Encoding: deflate`.

* Fixed headers formating for CORS Middleware Access-Control-Expose-Headers #436

* Fix adding multiple response headers #446

* Client includes port in HOST header when it is not default(e.g. not 80 and 443). #448

* Panic during access without routing being set #452

* Fixed http/2 error handling

### Deprecated

* `HttpServer::no_http2()` is deprecated, use `OpensslAcceptor::with_flags()` or
  `RustlsAcceptor::with_flags()` instead

* `HttpServer::listen_tls()`, `HttpServer::listen_ssl()`, `HttpServer::listen_rustls()` have been
  deprecated in favor of `HttpServer::listen_with()` with specific `acceptor`.

* `HttpServer::bind_tls()`, `HttpServer::bind_ssl()`, `HttpServer::bind_rustls()` have been
  deprecated in favor of `HttpServer::bind_with()` with specific `acceptor`.


## [0.7.3] - 2018-08-01

### Added

* Support HTTP/2 with rustls #36

* Allow TestServer to open a websocket on any URL (TestServer::ws_at()) #433

### Fixed

* Fixed failure 0.1.2 compatibility

* Do not override HOST header for client request #428

* Gz streaming, use `flate2::write::GzDecoder` #228

* HttpRequest::url_for is not working with scopes #429

* Fixed headers' formating for CORS Middleware `Access-Control-Expose-Headers` header value to HTTP/1.1 & HTTP/2 spec-compliant format #436


## [0.7.2] - 2018-07-26

### Added

* Add implementation of `FromRequest<S>` for `Option<T>` and `Result<T, Error>`

* Allow to handle application prefix, i.e. allow to handle `/app` path
  for application with `/app` prefix.
  Check [`App::prefix()`](https://actix.rs/actix-web/actix_web/struct.App.html#method.prefix)
  api doc.

* Add `CookieSessionBackend::http_only` method to set `HttpOnly` directive of cookies

### Changed

* Upgrade to cookie 0.11

* Removed the timestamp from the default logger middleware

### Fixed

* Missing response header "content-encoding" #421

* Fix stream draining for http/2 connections #290


## [0.7.1] - 2018-07-21

### Fixed

* Fixed default_resource 'not yet implemented' panic #410


## [0.7.0] - 2018-07-21

### Added

* Add `fs::StaticFileConfig` to provide means of customizing static
  file services. It allows to map `mime` to `Content-Disposition`,
  specify whether to use `ETag` and `Last-Modified` and allowed methods.

* Add `.has_prefixed_resource()` method to `router::ResourceInfo`
  for route matching with prefix awareness

* Add `HttpMessage::readlines()` for reading line by line.

* Add `ClientRequestBuilder::form()` for sending `application/x-www-form-urlencoded` requests.

* Add method to configure custom error handler to `Form` extractor.

* Add methods to `HttpResponse` to retrieve, add, and delete cookies

* Add `.set_content_type()` and `.set_content_disposition()` methods
  to `fs::NamedFile` to allow overriding the values inferred by default

* Add `fs::file_extension_to_mime()` helper function to get the MIME
  type for a file extension

* Add `.content_disposition()` method to parse Content-Disposition of
  multipart fields

* Re-export `actix::prelude::*` as `actix_web::actix` module.

* `HttpRequest::url_for_static()` for a named route with no variables segments

* Propagation of the application's default resource to scopes that haven't set a default resource.


### Changed

* Min rustc version is 1.26

* Use tokio instead of tokio-core

* `CookieSessionBackend` sets percent encoded cookies for outgoing HTTP messages.

* Became possible to use enums with query extractor.
  Issue [#371](https://github.com/actix/actix-web/issues/371).
  [Example](https://github.com/actix/actix-web/blob/master/tests/test_handlers.rs#L94-L134)

* `HttpResponse::into_builder()` now moves cookies into the builder
  instead of dropping them

* For safety and performance reasons `Handler::handle()` uses `&self` instead of `&mut self`

* `Handler::handle()` uses `&HttpRequest` instead of `HttpRequest`

* Added header `User-Agent: Actix-web/<current_version>` to default headers when building a request

* port `Extensions` type from http create, we don't need `Send + Sync`

* `HttpRequest::query()` returns `Ref<HashMap<String, String>>`

* `HttpRequest::cookies()` returns `Ref<Vec<Cookie<'static>>>`

* `StaticFiles::new()` returns `Result<StaticFiles<S>, Error>` instead of `StaticFiles<S>`

* `StaticFiles` uses the default handler if the file does not exist


### Removed

* Remove `Route::with2()` and `Route::with3()` use tuple of extractors instead.

* Remove `HttpMessage::range()`


## [0.6.15] - 2018-07-11

### Fixed

* Fix h2 compatibility #352

* Fix duplicate tail of StaticFiles with index_file. #344


## [0.6.14] - 2018-06-21

### Added

* Allow to disable masking for websockets client

### Fixed

* SendRequest execution fails with the "internal error: entered unreachable code" #329


## [0.6.13] - 2018-06-11

* http/2 end-of-frame is not set if body is empty bytes #307

* InternalError can trigger memory unsafety #301


## [0.6.12] - 2018-06-08

### Added

* Add `Host` filter #287

* Allow to filter applications

* Improved failure interoperability with downcasting #285

* Allow to use custom resolver for `ClientConnector`


## [0.6.11] - 2018-06-05

* Support chunked encoding for UrlEncoded body #262

* `HttpRequest::url_for()` for a named route with no variables segments #265

* `Middleware::response()` is not invoked if error result was returned by another `Middleware::start()` #255

* CORS: Do not validate Origin header on non-OPTION requests #271

* Fix multipart upload "Incomplete" error #282


## [0.6.10] - 2018-05-24

### Added

* Allow to use path without trailing slashes for scope registration #241

* Allow to set encoding for exact NamedFile #239

### Fixed

* `TestServer::post()` actually sends `GET` request #240


## 0.6.9 (2018-05-22)

* Drop connection if request's payload is not fully consumed #236

* Fix streaming response with body compression


## 0.6.8 (2018-05-20)

* Fix scope resource path extractor #234

* Re-use tcp listener on pause/resume


## 0.6.7 (2018-05-17)

* Fix compilation with --no-default-features


## 0.6.6 (2018-05-17)

* Panic during middleware execution #226

* Add support for listen_tls/listen_ssl #224

* Implement extractor for `Session`

* Ranges header support for NamedFile #60


## 0.6.5 (2018-05-15)

* Fix error handling during request decoding #222


## 0.6.4 (2018-05-11)

* Fix segfault in ServerSettings::get_response_builder()


## 0.6.3 (2018-05-10)

* Add `Router::with_async()` method for async handler registration.

* Added error response functions for 501,502,503,504

* Fix client request timeout handling


## 0.6.2 (2018-05-09)

* WsWriter trait is optional.


## 0.6.1 (2018-05-08)

* Fix http/2 payload streaming #215

* Fix connector's default `keep-alive` and `lifetime` settings #212

* Send `ErrorNotFound` instead of `ErrorBadRequest` when path extractor fails #214

* Allow to exclude certain endpoints from logging #211


## 0.6.0 (2018-05-08)

* Add route scopes #202

* Allow to use ssl and non-ssl connections at the same time #206

* Websocket CloseCode Empty/Status is ambiguous #193

* Add Content-Disposition to NamedFile #204

* Allow to access Error's backtrace object

* Allow to override files listing renderer for `StaticFiles` #203

* Various extractor usability improvements #207


## 0.5.6 (2018-04-24)

* Make flate2 crate optional #200


## 0.5.5 (2018-04-24)

* Fix panic when Websocket is closed with no error code #191

* Allow to use rust backend for flate2 crate #199

## 0.5.4 (2018-04-19)

* Add identity service middleware

* Middleware response() is not invoked if there was an error in async handler #187

* Use Display formatting for InternalError Display implementation #188


## 0.5.3 (2018-04-18)

* Impossible to quote slashes in path parameters #182


## 0.5.2 (2018-04-16)

* Allow to configure StaticFiles's CpuPool, via static method or env variable

* Add support for custom handling of Json extractor errors #181

* Fix StaticFiles does not support percent encoded paths #177

* Fix Client Request with custom Body Stream halting on certain size requests #176


## 0.5.1 (2018-04-12)

* Client connector provides stats, `ClientConnector::stats()`

* Fix end-of-stream handling in parse_payload #173

* Fix StaticFiles generate a lot of threads #174


## 0.5.0 (2018-04-10)

* Type-safe path/query/form parameter handling, using serde #70

* HttpResponse builder's methods  `.body()`, `.finish()`, `.json()`
  return `HttpResponse` instead of `Result`

* Use more ergonomic `actix_web::Error` instead of `http::Error` for `ClientRequestBuilder::body()`

* Added `signed` and `private` `CookieSessionBackend`s

* Added `HttpRequest::resource()`, returns current matched resource

* Added `ErrorHandlers` middleware

* Fix router cannot parse Non-ASCII characters in URL #137

* Fix client connection pooling

* Fix long client urls #129

* Fix panic on invalid URL characters #130

* Fix logger request duration calculation #152

* Fix prefix and static file serving #168


## 0.4.10 (2018-03-20)

* Use `Error` instead of `InternalError` for `error::ErrorXXXX` methods

* Allow to set client request timeout

* Allow to set client websocket handshake timeout

* Refactor `TestServer` configuration

* Fix server websockets big payloads support

* Fix http/2 date header generation


## 0.4.9 (2018-03-16)

* Allow to disable http/2 support

* Wake payload reading task when data is available

* Fix server keep-alive handling

* Send Query Parameters in client requests #120

* Move brotli encoding to a feature

* Add option of default handler for `StaticFiles` handler #57

* Add basic client connection pooling


## 0.4.8 (2018-03-12)

* Allow to set read buffer capacity for server request

* Handle WouldBlock error for socket accept call


## 0.4.7 (2018-03-11)

* Fix panic on unknown content encoding

* Fix connection get closed too early

* Fix streaming response handling for http/2

* Better sleep on error support


## 0.4.6 (2018-03-10)

* Fix client cookie handling

* Fix json content type detection

* Fix CORS middleware #117

* Optimize websockets stream support


## 0.4.5 (2018-03-07)

* Fix compression #103 and #104

* Fix client cookie handling #111

* Non-blocking processing of a `NamedFile`

* Enable compression support for `NamedFile`

* Better support for `NamedFile` type

* Add `ResponseError` impl for `SendRequestError`. This improves ergonomics of the client.

* Add native-tls support for client

* Allow client connection timeout to be set #108

* Allow to use std::net::TcpListener for HttpServer

* Handle panics in worker threads


## 0.4.4 (2018-03-04)

* Allow to use Arc<Vec<u8>> as response/request body

* Fix handling of requests with an encoded body with a length > 8192 #93

## 0.4.3 (2018-03-03)

* Fix request body read bug

* Fix segmentation fault #79

* Set reuse address before bind #90


## 0.4.2 (2018-03-02)

* Better naming for websockets implementation

* Add `Pattern::with_prefix()`, make it more usable outside of actix

* Add csrf middleware for filter for cross-site request forgery #89

* Fix disconnect on idle connections


## 0.4.1 (2018-03-01)

* Rename `Route::p()` to `Route::filter()`

* Better naming for http codes

* Fix payload parse in situation when socket data is not ready.

* Fix Session mutable borrow lifetime #87


## 0.4.0 (2018-02-28)

* Actix 0.5 compatibility

* Fix request json/urlencoded loaders

* Simplify HttpServer type definition

* Added HttpRequest::encoding() method

* Added HttpRequest::mime_type() method

* Added HttpRequest::uri_mut(), allows to modify request uri

* Added StaticFiles::index_file()

* Added http client

* Added websocket client

* Added TestServer::ws(), test websockets client

* Added TestServer http client support

* Allow to override content encoding on application level


## 0.3.3 (2018-01-25)

* Stop processing any events after context stop

* Re-enable write back-pressure for h1 connections

* Refactor HttpServer::start_ssl() method

* Upgrade openssl to 0.10


## 0.3.2 (2018-01-21)

* Fix HEAD requests handling

* Log request processing errors

* Always enable content encoding if encoding explicitly selected

* Allow multiple Applications on a single server with different state #49

* CORS middleware: allowed_headers is defaulting to None #50


## 0.3.1 (2018-01-13)

* Fix directory entry path #47

* Do not enable chunked encoding for HTTP/1.0

* Allow explicitly disable chunked encoding


## 0.3.0 (2018-01-12)

* HTTP/2 Support

* Refactor streaming responses

* Refactor error handling

* Asynchronous middlewares

* Refactor logger middleware

* Content compression/decompression (br, gzip, deflate)

* Server multi-threading

* Graceful shutdown support


## 0.2.1 (2017-11-03)

* Allow to start tls server with `HttpServer::serve_tls`

* Export `Frame` enum

* Add conversion impl from `HttpResponse` and `BinaryBody` to a `Frame`


## 0.2.0 (2017-10-30)

* Do not use `http::Uri` as it can not parse some valid paths

* Refactor response `Body`

* Refactor `RouteRecognizer` usability

* Refactor `HttpContext::write`

* Refactor `Payload` stream

* Re-use `BinaryBody` for `Frame::Payload`

* Stop http actor on `write_eof`

* Fix disconnection handling.


## 0.1.0 (2017-10-23)

* First release
