# Changelog

## Unreleased

## 4.8.0

### Added

- Add `web::Html` responder.
- Add `HttpRequest::full_url()` method to get the complete URL of the request.

### Fixed

- Always remove port from return value of `ConnectionInfo::realip_remote_addr()` when handling IPv6 addresses. from the `Forwarded` header.
- The `UrlencodedError::ContentType` variant (relevant to the `Form` extractor) now uses the 415 (Media Type Unsupported) status code in it's `ResponseError` implementation.
- Apply `HttpServer::max_connection_rate()` setting when using rustls v0.22 or v0.23.

## 4.7.0

### Added

- Add `#[scope]` macro.
- Add `middleware::Identity` type.
- Add `CustomizeResponder::add_cookie()` method.
- Add `guard::GuardContext::app_data()` method.
- Add `compat-routing-macros-force-pub` crate feature which (on-by-default) which, when disabled, causes handlers to inherit their attached function's visibility.
- Add `compat` crate feature group (on-by-default) which, when disabled, helps with transitioning to some planned v5.0 breaking changes, starting only with `compat-routing-macros-force-pub`.
- Implement `From<Box<dyn ResponseError>>` for `Error`.

## 4.6.0

### Added

- Add `unicode` crate feature (on-by-default) to switch between `regex` and `regex-lite` as a trade-off between full unicode support and binary size.
- Add `rustls-0_23` crate feature.
- Add `HttpServer::{bind_rustls_0_23, listen_rustls_0_23}()` builder methods.
- Add `HttpServer::tls_handshake_timeout()` builder method for `rustls-0_22` and `rustls-0_23`.

### Changed

- Update `brotli` dependency to `6`.
- Minimum supported Rust version (MSRV) is now 1.72.

### Fixed

- Avoid type confusion with `rustls` in some circumstances.

## 4.5.1

### Fixed

- Fix missing import when using enabling Rustls v0.22 support.

## 4.5.0

### Added

- Add `rustls-0_22` crate feature.
- Add `HttpServer::{bind_rustls_0_22, listen_rustls_0_22}()` builder methods.

## 4.4.1

### Changed

- Updated `zstd` dependency to `0.13`.
- Compression middleware now prefers brotli over zstd over gzip.

### Fixed

- Fix validation of `Json` extractor when `JsonConfig::validate_content_type()` is set to false.

## 4.4.0

### Added

- Add `HttpServer::{bind, listen}_auto_h2c()` methods behind new `http2` crate feature.
- Add `HttpServer::{bind, listen}_rustls_021()` methods for Rustls v0.21 support behind new `rustls-0_21` crate feature.
- Add `Resource::{get, post, etc...}` methods for more concisely adding routes that don't need additional guards.
- Add `web::Payload::to_bytes[_limited]()` helper methods.
- Add missing constructors on `HttpResponse` for several status codes.
- Add `http::header::ContentLength` typed header.
- Implement `Default` for `web::Data`.
- Implement `serde::Deserialize` for `web::Data`.
- Add `rustls-0_20` crate feature, which the existing `rustls` feature now aliases.

### Changed

- Handler functions can now receive up to 16 extractor parameters.
- The `Compress` middleware no longer compresses image or video content.
- Hide sensitive header values in `HttpRequest`'s `Debug` output.
- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 4.3.1

### Added

- Add support for custom methods with the `#[route]` macro. [#2969]

[#2969]: https://github.com/actix/actix-web/pull/2969

## 4.3.0

### Added

- Add `ContentDisposition::attachment()` constructor. [#2867]
- Add `ErrorHandlers::default_handler()` (as well as `default_handler_{server, client}()`) to make registering handlers for groups of response statuses easier. [#2784]
- Add `Logger::custom_response_replace()`. [#2631]
- Add rudimentary redirection service at `web::redirect()` / `web::Redirect`. [#1961]
- Add `guard::Acceptable` for matching against `Accept` header MIME types. [#2265]
- Add fallible versions of `test` helpers: `try_call_service()`, `try_call_and_read_body_json()`, `try_read_body()`, and `try_read_body_json()`. [#2961]

### Fixed

- Add `Allow` header to `Resource`'s default responses when no routes are matched. [#2949]

[#1961]: https://github.com/actix/actix-web/pull/1961
[#2265]: https://github.com/actix/actix-web/pull/2265
[#2631]: https://github.com/actix/actix-web/pull/2631
[#2784]: https://github.com/actix/actix-web/pull/2784
[#2867]: https://github.com/actix/actix-web/pull/2867
[#2949]: https://github.com/actix/actix-web/pull/2949
[#2961]: https://github.com/actix/actix-web/pull/2961

## 4.2.1

### Fixed

- Bump minimum version of `actix-http` dependency to fix compatibility issue. [#2871]

[#2871]: https://github.com/actix/actix-web/pull/2871

## 4.2.0

### Added

- Add `#[routes]` macro to support multiple paths for one handler. [#2718]
- Add `ServiceRequest::{parts, request}()` getter methods. [#2786]
- Add configuration options for TLS handshake timeout via `HttpServer::{rustls, openssl}_with_config` methods. [#2752]

### Changed

- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

[#2718]: https://github.com/actix/actix-web/pull/2718
[#2752]: https://github.com/actix/actix-web/pull/2752
[#2786]: https://github.com/actix/actix-web/pull/2786

## 4.1.0

### Added

- Add `ServiceRequest::extract()` to make it easier to use extractors when writing middlewares. [#2647]
- Add `Route::wrap()` to allow individual routes to use middleware. [#2725]
- Add `ServiceConfig::default_service()`. [#2338] [#2743]
- Implement `ResponseError` for `std::convert::Infallible`

### Changed

- Minimum supported Rust version (MSRV) is now 1.56 due to transitive `hashbrown` dependency.

### Fixed

- Clear connection-level data on `HttpRequest` drop. [#2742]

[#2338]: https://github.com/actix/actix-web/pull/2338
[#2647]: https://github.com/actix/actix-web/pull/2647
[#2725]: https://github.com/actix/actix-web/pull/2725
[#2742]: https://github.com/actix/actix-web/pull/2742
[#2743]: https://github.com/actix/actix-web/pull/2743

## 4.0.1

### Fixed

- Use stable version in readme example.

## 4.0.0

### Dependencies

- Updated `actix-*` to Tokio v1-based versions. [#1813]
- Updated `actix-web-codegen` to `4.0.0`.
- Updated `cookie` to `0.16`. [#2555]
- Updated `language-tags` to `0.3`.
- Updated `rand` to `0.8`.
- Updated `rustls` to `0.20`. [#2414]
- Updated `tokio` to `1`.

### Added

- Crate Features:
  - `cookies`; enabled by default. [#2619]
  - `compress-brotli`; enabled by default. [#2618]
  - `compress-gzip`; enabled by default. [#2618]
  - `compress-zstd`; enabled by default. [#2618]
  - `macros`; enables routing and runtime macros, enabled by default. [#2619]
- Types:
  - `CustomizeResponder` for customizing response. [#2510]
  - `dev::ServerHandle` re-export from `actix-server`. [#2442]
  - `dev::ServiceFactory` re-export from `actix-service`. [#2325]
  - `guard::GuardContext` for use with the `Guard` trait. [#2552]
  - `http::header::AcceptEncoding` typed header. [#2482]
  - `http::header::Range` typed header. [#2485]
  - `http::KeepAlive` re-export from `actix-http`. [#2625]
  - `middleware::Compat` that boxes middleware types like `Logger` and `Compress` to be used with constrained type bounds. [#1865]
  - `web::Header` extractor for extracting typed HTTP headers in handlers. [#2094]
- Methods:
  - `dev::ServiceRequest::guard_ctx()` for obtaining a guard context. [#2552]
  - `dev::ServiceRequest::parts_mut()`. [#2177]
  - `dev::ServiceResponse::map_into_{left,right}_body()` and `HttpResponse::map_into_boxed_body()`. [#2468]
  - `Either<web::Json<T>, web::Form<T>>::into_inner()` which returns the inner type for whichever variant was created. Also works for `Either<web::Form<T>, web::Json<T>>`. [#1894]
  - `http::header::AcceptLanguage::{ranked, preference}()`. [#2480]
  - `HttpResponse::add_removal_cookie()`. [#2586]
  - `HttpResponse::map_into_{left,right}_body()` and `HttpResponse::map_into_boxed_body()`. [#2468]
  - `HttpServer::worker_max_blocking_threads` for setting block thread pool. [#2200]
  - `middleware::Logger::log_target()` to allow customize. [#2594]
  - `Responder::customize()` trait method that wraps responder in `CustomizeResponder`. [#2510]
  - `Route::service()` for using hand-written services as handlers. [#2262]
  - `ServiceResponse::into_parts()`. [#2499]
  - `TestServer::client_headers()` method. [#2097]
  - `web::ServiceConfig::configure()` to allow easy nesting of configuration functions. [#1988]
- Trait Implementations:
  - Implement `Debug` for `DefaultHeaders`. [#2510]
  - Implement `FromRequest` for `ConnectionInfo` and `PeerAddr`. [#2263]
  - Implement `FromRequest` for `Method`. [#2263]
  - Implement `FromRequest` for `Uri`. [#2263]
  - Implement `Hash` for `http::header::Encoding`. [#2501]
  - Implement `Responder` for `Vec<u8>`. [#2625]
- Misc:
  - `#[actix_web::test]` macro for setting up tests with a runtime. [#2409]
  - Enable registering a vec of services of the same type to `App` [#1933]
  - Add `services!` macro for helping register multiple services to `App`. [#1933]
  - Option to allow `Json` extractor to work without a `Content-Type` header present. [#2362]
  - Connection data set through the `HttpServer::on_connect` callback is now accessible only from the new `HttpRequest::conn_data()` and `ServiceRequest::conn_data()` methods. [#2491]

### Changed

- Functions:
  - `guard::fn_guard` functions now receives a `&GuardContext`. [#2552]
  - `guard::Not` is now generic over the type of guard it wraps. [#2552]
  - `test::{call_service, read_response, read_response_json, send_request}()` now receive a `&Service`. [#1905]
  - Some guard functions now return `impl Guard` and their concrete types are made private: `guard::Header` and all the method guards. [#2552]
  - Rename `test::{default_service => status_service}()`. Old name is deprecated. [#2518]
  - Rename `test::{read_response_json => call_and_read_body_json}()`. Old name is deprecated. [#2518]
  - Rename `test::{read_response => call_and_read_body}()`. Old name is deprecated. [#2518]
- Traits:
  - `guard::Guard::check` now receives a `&GuardContext`. [#2552]
  - `FromRequest::Config` associated type was removed. [#2233]
  - `Responder` trait has been reworked and now `Response`/`HttpResponse` synchronously, making it simpler and more performant. [#1891]
  - Rename `Factory` trait to `Handler`. [#1852]
- Types:
  - `App`'s `B` (body) type parameter been removed. As a result, `App`s can be returned from functions now. [#2493]
  - `Compress` middleware's response type is now `EitherBody<Encoder<B>>`. [#2448]
  - `error::BlockingError` is now a unit struct. It's now only triggered when blocking thread pool has shutdown. [#1957]
  - `ErrorHandlerResponse`'s response variants now use `ServiceResponse<EitherBody<B>>`. [#2515]
  - `ErrorHandlers` middleware's response types now use `ServiceResponse<EitherBody<B>>`. [#2515]
  - `http::header::Encoding` now only represents `Content-Encoding` types. [#2501]
  - `middleware::Condition` gained a broader middleware compatibility. [#2635]
  - `Resource` no longer require service body type to be boxed. [#2526]
  - `Scope` no longer require service body type to be boxed. [#2523]
  - `web::Path`s inner field is now private. [#1894]
  - `web::Payload`'s inner field is now private. [#2384]
  - Error enums are now marked `#[non_exhaustive]`. [#2148]
- Enum Variants:
  - `Either` now uses `Left`/`Right` variants (instead of `A`/`B`) [#1894]
  - Include size and limits in `JsonPayloadError::Overflow`. [#2162]
- Methods:
  - `App::data()` is deprecated; `App::app_data()` should be preferred. [#2271]
  - `dev::JsonBody::new()` returns a default limit of 32kB to be consistent with `JsonConfig` and the default behaviour of the `web::Json<T>` extractor. [#2010]
  - `dev::ServiceRequest::{into_parts, from_parts}()` can no longer fail. [#1893]
  - `dev::ServiceRequest::from_request` can no longer fail. [#1893]
  - `dev::ServiceResponse::error_response()` now uses body type of `BoxBody`. [#2201]
  - `dev::ServiceResponse::map_body()` closure receives and returns `B` instead of `ResponseBody<B>`. [#2201]
  - `http::header::ContentType::html()` now produces `text/html; charset=utf-8` instead of `text/html`. [#2423]
  - `HttpRequest::url_for`'s constructed URLs no longer contain query or fragment. [#2430]
  - `HttpResponseBuilder::json()` can now receive data by value and reference. [#1903]
  - `HttpServer::{listen_rustls, bind_rustls}()` now honor the ALPN protocols in the configuration parameter. [#2226]
  - `middleware::NormalizePath()` now will not try to normalize URIs with no valid path [#2246]
  - `test::TestRequest::param()` now accepts more than just static strings. [#2172]
  - `web::Data::into_inner()` and `Data::get_ref()` no longer require `T: Sized`. [#2403]
  - Rename `HttpServer::{client_timeout => client_request_timeout}()`. [#2611]
  - Rename `HttpServer::{client_shutdown => client_disconnect_timeout}()`. [#2611]
  - Rename `http::header::Accept::{mime_precedence => ranked}()`. [#2480]
  - Rename `http::header::Accept::{mime_preference => preference}()`. [#2480]
  - Rename `middleware::DefaultHeaders::{content_type => add_content_type}()`. [#1875]
  - Rename `dev::ConnectionInfo::{remote_addr => peer_addr}`, deprecating the old name. [#2554]
- Trait Implementations:
  - `HttpResponse` can now be used as a `Responder` with any body type. [#2567]
- Misc:
  - Maximum number of handler extractors has increased to 12. [#2582]
  - The default `TrailingSlash` behavior is now `Trim`, in line with existing documentation. See migration guide for implications. [#1875]
  - `Result` extractor wrapper can now convert error types. [#2581]
  - Compress middleware will return `406 Not Acceptable` when no content encoding is acceptable to the client. [#2344]
  - Adjusted default JSON payload limit to 2MB (from 32kb). [#2162]
  - All error trait bounds in server service builders have changed from `Into<Error>` to `Into<Response<BoxBody>>`. [#2253]
  - All error trait bounds in message body and stream impls changed from `Into<Error>` to `Into<Box<dyn std::error::Error>>`. [#2253]
  - Improve spec compliance of `dev::ConnectionInfo` extractor. [#2282]
  - Associated types in `FromRequest` implementation for `Option` and `Result` have changed. [#2581]
  - Reduce the level from `error` to `debug` for the log line that is emitted when a `500 Internal Server Error` is built using `HttpResponse::from_error`. [#2201]
  - Minimum supported Rust version (MSRV) is now 1.54.

### Fixed

- Auto-negotiation of content encoding is more fault-tolerant when using the `Compress` middleware. [#2501]
- Scope and Resource middleware can access data items set on their own layer. [#2288]
- Multiple calls to `App::data()` with the same type now keeps the latest call's data. [#1906]
- Typed headers containing lists that require one or more items now enforce this minimum. [#2482]
- `dev::ConnectionInfo::peer_addr` will no longer return the port number. [#2554]
- `dev::ConnectionInfo::realip_remote_addr` will no longer return the port number if sourcing the IP from the peer's socket address. [#2554]
- Accept wildcard `*` items in `AcceptLanguage`. [#2480]
- Relax `Unpin` bound on `S` (stream) parameter of `HttpResponseBuilder::streaming`. [#2448]
- Fix quality parse error in `http::header::AcceptEncoding` typed header. [#2344]
- Double ampersand in `middleware::Logger` format is escaped correctly. [#2067]
- Added the underlying parse error to `test::read_body_json`'s panic message. [#1812]

### Security

- `cookie` upgrade addresses [`RUSTSEC-2020-0071`].

[`rustsec-2020-0071`]: https://rustsec.org/advisories/RUSTSEC-2020-0071.html

### Removed

- Crate Features:
  - `compress` feature. [#2065]
- Functions:
  - `test::load_stream` and `test::load_body`; replace usage with `body::to_bytes`. [#2518]
  - `test::start_with`; moved to new `actix-test` crate. [#2112]
  - `test::start`; moved to new `actix-test` crate. [#2112]
  - `test::unused_addr`; moved to new `actix-test` crate. [#2112]
- Traits:
  - `BodyEncoding`; signalling content encoding is now only done via the `Content-Encoding` header. [#2565]
- Types:
  - `dev::{BodySize, MessageBody, SizedStream}` re-exports; they are exposed through the `body` module. [#2468]
  - `EitherExtractError` direct export. [#2510]
  - `rt::{Arbiter, ArbiterHandle}` re-exports. [#2619]
  - `test::TestServer`; moved to new `actix-test` crate. [#2112]
  - `test::TestServerConfig`; moved to new `actix-test` crate. [#2112]
  - `web::HttpRequest` re-export. [#2663]
  - `web::HttpResponse` re-export. [#2663]
- Methods:
  - `AppService::set_service_data`; for custom HTTP service factories adding application data, use the layered data model by calling `ServiceRequest::add_data_container` when handling requests instead. [#1906]
  - `dev::ConnectionInfo::get`. [#2487]
  - `dev::ServiceResponse::checked_expr`. [#2401]
  - `HttpRequestBuilder::del_cookie`. [#2591]
  - `HttpResponse::take_body` and old `HttpResponse::into_body` method that casted body type. [#2201]
  - `HttpResponseBuilder::json2()`. [#1903]
  - `middleware::Compress::new`; restricting compression algorithm is done through feature flags. [#2501]
  - `test::TestRequest::with_header()`; use `test::TestRequest::default().insert_header()`. [#1869]
- Trait Implementations:
  - Implementation of `From<either::Either>` for `Either` crate. [#2516]
  - Implementation of `Future` for `HttpResponse`. [#2601]
- Misc:
  - The `client` module was removed; use the `awc` crate directly. [871ca5e4]
  - `middleware::{normalize, err_handlers}` modules; all necessary middleware types are now exposed in the `middleware` module.

[#1812]: https://github.com/actix/actix-web/pull/1812
[#1813]: https://github.com/actix/actix-web/pull/1813
[#1852]: https://github.com/actix/actix-web/pull/1852
[#1865]: https://github.com/actix/actix-web/pull/1865
[#1869]: https://github.com/actix/actix-web/pull/1869
[#1875]: https://github.com/actix/actix-web/pull/1875
[#1878]: https://github.com/actix/actix-web/pull/1878
[#1891]: https://github.com/actix/actix-web/pull/1891
[#1893]: https://github.com/actix/actix-web/pull/1893
[#1894]: https://github.com/actix/actix-web/pull/1894
[#1903]: https://github.com/actix/actix-web/pull/1903
[#1905]: https://github.com/actix/actix-web/pull/1905
[#1906]: https://github.com/actix/actix-web/pull/1906
[#1933]: https://github.com/actix/actix-web/pull/1933
[#1957]: https://github.com/actix/actix-web/pull/1957
[#1957]: https://github.com/actix/actix-web/pull/1957
[#1981]: https://github.com/actix/actix-web/pull/1981
[#1988]: https://github.com/actix/actix-web/pull/1988
[#2010]: https://github.com/actix/actix-web/pull/2010
[#2065]: https://github.com/actix/actix-web/pull/2065
[#2067]: https://github.com/actix/actix-web/pull/2067
[#2093]: https://github.com/actix/actix-web/pull/2093
[#2094]: https://github.com/actix/actix-web/pull/2094
[#2097]: https://github.com/actix/actix-web/pull/2097
[#2112]: https://github.com/actix/actix-web/pull/2112
[#2148]: https://github.com/actix/actix-web/pull/2148
[#2162]: https://github.com/actix/actix-web/pull/2162
[#2172]: https://github.com/actix/actix-web/pull/2172
[#2177]: https://github.com/actix/actix-web/pull/2177
[#2200]: https://github.com/actix/actix-web/pull/2200
[#2201]: https://github.com/actix/actix-web/pull/2201
[#2201]: https://github.com/actix/actix-web/pull/2201
[#2233]: https://github.com/actix/actix-web/pull/2233
[#2246]: https://github.com/actix/actix-web/pull/2246
[#2250]: https://github.com/actix/actix-web/pull/2250
[#2253]: https://github.com/actix/actix-web/pull/2253
[#2262]: https://github.com/actix/actix-web/pull/2262
[#2263]: https://github.com/actix/actix-web/pull/2263
[#2271]: https://github.com/actix/actix-web/pull/2271
[#2282]: https://github.com/actix/actix-web/pull/2282
[#2288]: https://github.com/actix/actix-web/pull/2288
[#2325]: https://github.com/actix/actix-web/pull/2325
[#2344]: https://github.com/actix/actix-web/pull/2344
[#2362]: https://github.com/actix/actix-web/pull/2362
[#2379]: https://github.com/actix/actix-web/pull/2379
[#2384]: https://github.com/actix/actix-web/pull/2384
[#2401]: https://github.com/actix/actix-web/pull/2401
[#2403]: https://github.com/actix/actix-web/pull/2403
[#2409]: https://github.com/actix/actix-web/pull/2409
[#2414]: https://github.com/actix/actix-web/pull/2414
[#2423]: https://github.com/actix/actix-web/pull/2423
[#2430]: https://github.com/actix/actix-web/pull/2430
[#2442]: https://github.com/actix/actix-web/pull/2442
[#2446]: https://github.com/actix/actix-web/pull/2446
[#2448]: https://github.com/actix/actix-web/pull/2448
[#2468]: https://github.com/actix/actix-web/pull/2468
[#2474]: https://github.com/actix/actix-web/pull/2474
[#2480]: https://github.com/actix/actix-web/pull/2480
[#2482]: https://github.com/actix/actix-web/pull/2482
[#2484]: https://github.com/actix/actix-web/pull/2484
[#2485]: https://github.com/actix/actix-web/pull/2485
[#2487]: https://github.com/actix/actix-web/pull/2487
[#2491]: https://github.com/actix/actix-web/pull/2491
[#2492]: https://github.com/actix/actix-web/pull/2492
[#2493]: https://github.com/actix/actix-web/pull/2493
[#2499]: https://github.com/actix/actix-web/pull/2499
[#2501]: https://github.com/actix/actix-web/pull/2501
[#2510]: https://github.com/actix/actix-web/pull/2510
[#2515]: https://github.com/actix/actix-web/pull/2515
[#2516]: https://github.com/actix/actix-web/pull/2516
[#2518]: https://github.com/actix/actix-web/pull/2518
[#2523]: https://github.com/actix/actix-web/pull/2523
[#2526]: https://github.com/actix/actix-web/pull/2526
[#2552]: https://github.com/actix/actix-web/pull/2552
[#2554]: https://github.com/actix/actix-web/pull/2554
[#2555]: https://github.com/actix/actix-web/pull/2555
[#2565]: https://github.com/actix/actix-web/pull/2565
[#2567]: https://github.com/actix/actix-web/pull/2567
[#2569]: https://github.com/actix/actix-web/pull/2569
[#2581]: https://github.com/actix/actix-web/pull/2581
[#2582]: https://github.com/actix/actix-web/pull/2582
[#2584]: https://github.com/actix/actix-web/pull/2584
[#2585]: https://github.com/actix/actix-web/pull/2585
[#2586]: https://github.com/actix/actix-web/pull/2586
[#2591]: https://github.com/actix/actix-web/pull/2591
[#2594]: https://github.com/actix/actix-web/pull/2594
[#2601]: https://github.com/actix/actix-web/pull/2601
[#2611]: https://github.com/actix/actix-web/pull/2611
[#2619]: https://github.com/actix/actix-web/pull/2619
[#2625]: https://github.com/actix/actix-web/pull/2625
[#2635]: https://github.com/actix/actix-web/pull/2635
[#2659]: https://github.com/actix/actix-web/pull/2659
[#2663]: https://github.com/actix/actix-web/pull/2663
[871ca5e4]: https://github.com/actix/actix-web/commit/871ca5e4ae2bdc22d1ea02701c2992fa8d04aed7

<details>
<summary>4.0.0 Pre-Releases</summary>

## 4.0.0-rc.3

### Changed

- `middleware::Condition` gained a broader compatibility; `Compat` is needed in fewer cases. [#2635]

### Added

- Implement `Responder` for `Vec<u8>`. [#2625]
- Re-export `KeepAlive` in `http` mod. [#2625]

[#2625]: https://github.com/actix/actix-web/pull/2625
[#2635]: https://github.com/actix/actix-web/pull/2635

## 4.0.0-rc.2

### Added

- On-by-default `macros` feature flag to enable routing and runtime macros. [#2619]

### Removed

- `rt::{Arbiter, ArbiterHandle}` re-exports. [#2619]

[#2619]: https://github.com/actix/actix-web/pull/2619

## 4.0.0-rc.1

### Changed

- Rename `HttpServer::{client_timeout => client_request_timeout}`. [#2611]
- Rename `HttpServer::{client_shutdown => client_disconnect_timeout}`. [#2611]

### Removed

- `impl Future for HttpResponse`. [#2601]

[#2601]: https://github.com/actix/actix-web/pull/2601
[#2611]: https://github.com/actix/actix-web/pull/2611

## 4.0.0-beta.21

### Added

- `HttpResponse::add_removal_cookie`. [#2586]
- `Logger::log_target`. [#2594]

### Removed

- `HttpRequest::req_data[_mut]()`; request-local data is still available through `.extensions()`. [#2585]
- `HttpRequestBuilder::del_cookie`. [#2591]

[#2585]: https://github.com/actix/actix-web/pull/2585
[#2586]: https://github.com/actix/actix-web/pull/2586
[#2591]: https://github.com/actix/actix-web/pull/2591
[#2594]: https://github.com/actix/actix-web/pull/2594

## 4.0.0-beta.20

### Added

- `GuardContext::header` [#2569]
- `ServiceConfig::configure` to allow easy nesting of configuration functions. [#1988]

### Changed

- `HttpResponse` can now be used as a `Responder` with any body type. [#2567]
- `Result` extractor wrapper can now convert error types. [#2581]
- Associated types in `FromRequest` impl for `Option` and `Result` has changed. [#2581]
- Maximum number of handler extractors has increased to 12. [#2582]
- Removed bound `<B as MessageBody>::Error: Debug` in test utility functions in order to support returning opaque apps. [#2584]

[#1988]: https://github.com/actix/actix-web/pull/1988
[#2567]: https://github.com/actix/actix-web/pull/2567
[#2569]: https://github.com/actix/actix-web/pull/2569
[#2581]: https://github.com/actix/actix-web/pull/2581
[#2582]: https://github.com/actix/actix-web/pull/2582
[#2584]: https://github.com/actix/actix-web/pull/2584

## 4.0.0-beta.19

### Added

- `impl Hash` for `http::header::Encoding`. [#2501]
- `AcceptEncoding::negotiate()`. [#2501]

### Changed

- `AcceptEncoding::preference` now returns `Option<Preference<Encoding>>`. [#2501]
- Rename methods `BodyEncoding::{encoding => encode_with, get_encoding => preferred_encoding}`. [#2501]
- `http::header::Encoding` now only represents `Content-Encoding` types. [#2501]

### Fixed

- Auto-negotiation of content encoding is more fault-tolerant when using the `Compress` middleware. [#2501]

### Removed

- `Compress::new`; restricting compression algorithm is done through feature flags. [#2501]
- `BodyEncoding` trait; signalling content encoding is now only done via the `Content-Encoding` header. [#2565]

[#2501]: https://github.com/actix/actix-web/pull/2501
[#2565]: https://github.com/actix/actix-web/pull/2565

## 4.0.0-beta.18

### Changed

- Update `cookie` dependency (re-exported) to `0.16`. [#2555]
- Minimum supported Rust version (MSRV) is now 1.54.

### Security

- `cookie` upgrade addresses [`RUSTSEC-2020-0071`].

[#2555]: https://github.com/actix/actix-web/pull/2555
[`rustsec-2020-0071`]: https://rustsec.org/advisories/RUSTSEC-2020-0071.html

## 4.0.0-beta.17

### Added

- `guard::GuardContext` for use with the `Guard` trait. [#2552]
- `ServiceRequest::guard_ctx` for obtaining a guard context. [#2552]

### Changed

- `Guard` trait now receives a `&GuardContext`. [#2552]
- `guard::fn_guard` functions now receives a `&GuardContext`. [#2552]
- Some guards now return `impl Guard` and their concrete types are made private: `guard::Header` and all the method guards. [#2552]
- The `Not` guard is now generic over the type of guard it wraps. [#2552]

### Fixed

- Rename `ConnectionInfo::{remote_addr => peer_addr}`, deprecating the old name. [#2554]
- `ConnectionInfo::peer_addr` will not return the port number. [#2554]
- `ConnectionInfo::realip_remote_addr` will not return the port number if sourcing the IP from the peer's socket address. [#2554]

[#2552]: https://github.com/actix/actix-web/pull/2552
[#2554]: https://github.com/actix/actix-web/pull/2554

## 4.0.0-beta.16

### Changed

- No longer require `Scope` service body type to be boxed. [#2523]
- No longer require `Resource` service body type to be boxed. [#2526]

[#2523]: https://github.com/actix/actix-web/pull/2523
[#2526]: https://github.com/actix/actix-web/pull/2526

## 4.0.0-beta.15

### Added

- Method on `Responder` trait (`customize`) for customizing responders and `CustomizeResponder` struct. [#2510]
- Implement `Debug` for `DefaultHeaders`. [#2510]

### Changed

- Align `DefaultHeader` method terminology, deprecating previous methods. [#2510]
- Response service types in `ErrorHandlers` middleware now use `ServiceResponse<EitherBody<B>>` to allow changing the body type. [#2515]
- Both variants in `ErrorHandlerResponse` now use `ServiceResponse<EitherBody<B>>`. [#2515]
- Rename `test::{default_service => simple_service}`. Old name is deprecated. [#2518]
- Rename `test::{read_response_json => call_and_read_body_json}`. Old name is deprecated. [#2518]
- Rename `test::{read_response => call_and_read_body}`. Old name is deprecated. [#2518]
- Relax body type and error bounds on test utilities. [#2518]

### Removed

- Top-level `EitherExtractError` export. [#2510]
- Conversion implementations for `either` crate. [#2516]
- `test::load_stream` and `test::load_body`; replace usage with `body::to_bytes`. [#2518]

[#2510]: https://github.com/actix/actix-web/pull/2510
[#2515]: https://github.com/actix/actix-web/pull/2515
[#2516]: https://github.com/actix/actix-web/pull/2516
[#2518]: https://github.com/actix/actix-web/pull/2518

## 4.0.0-beta.14

### Added

- Methods on `AcceptLanguage`: `ranked` and `preference`. [#2480]
- `AcceptEncoding` typed header. [#2482]
- `Range` typed header. [#2485]
- `HttpResponse::map_into_{left,right}_body` and `HttpResponse::map_into_boxed_body`. [#2468]
- `ServiceResponse::map_into_{left,right}_body` and `HttpResponse::map_into_boxed_body`. [#2468]
- Connection data set through the `HttpServer::on_connect` callback is now accessible only from the new `HttpRequest::conn_data()` and `ServiceRequest::conn_data()` methods. [#2491]
- `HttpRequest::{req_data,req_data_mut}`. [#2487]
- `ServiceResponse::into_parts`. [#2499]

### Changed

- Rename `Accept::{mime_precedence => ranked}`. [#2480]
- Rename `Accept::{mime_preference => preference}`. [#2480]
- Un-deprecate `App::data_factory`. [#2484]
- `HttpRequest::url_for` no longer constructs URLs with query or fragment components. [#2430]
- Remove `B` (body) type parameter on `App`. [#2493]
- Add `B` (body) type parameter on `Scope`. [#2492]
- Request-local data container is no longer part of a `RequestHead`. Instead it is a distinct part of a `Request`. [#2487]

### Fixed

- Accept wildcard `*` items in `AcceptLanguage`. [#2480]
- Re-exports `dev::{BodySize, MessageBody, SizedStream}`. They are exposed through the `body` module. [#2468]
- Typed headers containing lists that require one or more items now enforce this minimum. [#2482]

### Removed

- `ConnectionInfo::get`. [#2487]

[#2430]: https://github.com/actix/actix-web/pull/2430
[#2468]: https://github.com/actix/actix-web/pull/2468
[#2480]: https://github.com/actix/actix-web/pull/2480
[#2482]: https://github.com/actix/actix-web/pull/2482
[#2484]: https://github.com/actix/actix-web/pull/2484
[#2485]: https://github.com/actix/actix-web/pull/2485
[#2487]: https://github.com/actix/actix-web/pull/2487
[#2491]: https://github.com/actix/actix-web/pull/2491
[#2492]: https://github.com/actix/actix-web/pull/2492
[#2493]: https://github.com/actix/actix-web/pull/2493
[#2499]: https://github.com/actix/actix-web/pull/2499

## 4.0.0-beta.13

### Changed

- Update `actix-tls` to `3.0.0-rc.1`. [#2474]

[#2474]: https://github.com/actix/actix-web/pull/2474

## 4.0.0-beta.12

### Changed

- Compress middleware's response type is now `AnyBody<Encoder<B>>`. [#2448]

### Fixed

- Relax `Unpin` bound on `S` (stream) parameter of `HttpResponseBuilder::streaming`. [#2448]

### Removed

- `dev::ResponseBody` re-export; is function is replaced by the new `dev::AnyBody` enum. [#2446]

[#2446]: https://github.com/actix/actix-web/pull/2446
[#2448]: https://github.com/actix/actix-web/pull/2448

## 4.0.0-beta.11

### Added

- Re-export `dev::ServerHandle` from `actix-server`. [#2442]

### Changed

- `ContentType::html` now produces `text/html; charset=utf-8` instead of `text/html`. [#2423]
- Update `actix-server` to `2.0.0-beta.9`. [#2442]

[#2423]: https://github.com/actix/actix-web/pull/2423
[#2442]: https://github.com/actix/actix-web/pull/2442

## 4.0.0-beta.10

### Added

- Option to allow `Json` extractor to work without a `Content-Type` header present. [#2362]
- `#[actix_web::test]` macro for setting up tests with a runtime. [#2409]

### Changed

- Associated type `FromRequest::Config` was removed. [#2233]
- Inner field made private on `web::Payload`. [#2384]
- `Data::into_inner` and `Data::get_ref` no longer requires `T: Sized`. [#2403]
- Updated rustls to v0.20. [#2414]
- Minimum supported Rust version (MSRV) is now 1.52.

### Removed

- Useless `ServiceResponse::checked_expr` method. [#2401]

[#2233]: https://github.com/actix/actix-web/pull/2233
[#2362]: https://github.com/actix/actix-web/pull/2362
[#2384]: https://github.com/actix/actix-web/pull/2384
[#2401]: https://github.com/actix/actix-web/pull/2401
[#2403]: https://github.com/actix/actix-web/pull/2403
[#2409]: https://github.com/actix/actix-web/pull/2409
[#2414]: https://github.com/actix/actix-web/pull/2414

## 4.0.0-beta.9

### Added

- Re-export actix-service `ServiceFactory` in `dev` module. [#2325]

### Changed

- Compress middleware will return 406 Not Acceptable when no content encoding is acceptable to the client. [#2344]
- Move `BaseHttpResponse` to `dev::Response`. [#2379]
- Enable `TestRequest::param` to accept more than just static strings. [#2172]
- Minimum supported Rust version (MSRV) is now 1.51.

### Fixed

- Fix quality parse error in Accept-Encoding header. [#2344]
- Re-export correct type at `web::HttpResponse`. [#2379]

[#2172]: https://github.com/actix/actix-web/pull/2172
[#2325]: https://github.com/actix/actix-web/pull/2325
[#2344]: https://github.com/actix/actix-web/pull/2344
[#2379]: https://github.com/actix/actix-web/pull/2379

## 4.0.0-beta.8

### Added

- Add `ServiceRequest::parts_mut`. [#2177]
- Add extractors for `Uri` and `Method`. [#2263]
- Add extractors for `ConnectionInfo` and `PeerAddr`. [#2263]
- Add `Route::service` for using hand-written services as handlers. [#2262]

### Changed

- Change compression algorithm features flags. [#2250]
- Deprecate `App::data` and `App::data_factory`. [#2271]
- Smarter extraction of `ConnectionInfo` parts. [#2282]

### Fixed

- Scope and Resource middleware can access data items set on their own layer. [#2288]

[#2177]: https://github.com/actix/actix-web/pull/2177
[#2250]: https://github.com/actix/actix-web/pull/2250
[#2271]: https://github.com/actix/actix-web/pull/2271
[#2262]: https://github.com/actix/actix-web/pull/2262
[#2263]: https://github.com/actix/actix-web/pull/2263
[#2282]: https://github.com/actix/actix-web/pull/2282
[#2288]: https://github.com/actix/actix-web/pull/2288

## 4.0.0-beta.7

### Added

- `HttpServer::worker_max_blocking_threads` for setting block thread pool. [#2200]

### Changed

- Adjusted default JSON payload limit to 2MB (from 32kb) and included size and limits in the `JsonPayloadError::Overflow` error variant. [#2162]
- `ServiceResponse::error_response` now uses body type of `Body`. [#2201]
- `ServiceResponse::checked_expr` now returns a `Result`. [#2201]
- Update `language-tags` to `0.3`.
- `ServiceResponse::take_body`. [#2201]
- `ServiceResponse::map_body` closure receives and returns `B` instead of `ResponseBody<B>` types. [#2201]
- All error trait bounds in server service builders have changed from `Into<Error>` to `Into<Response<AnyBody>>`. [#2253]
- All error trait bounds in message body and stream impls changed from `Into<Error>` to `Into<Box<dyn std::error::Error>>`. [#2253]
- `HttpServer::{listen_rustls(), bind_rustls()}` now honor the ALPN protocols in the configuration parameter. [#2226]
- `middleware::normalize` now will not try to normalize URIs with no valid path [#2246]

### Removed

- `HttpResponse::take_body` and old `HttpResponse::into_body` method that casted body type. [#2201]

[#2162]: https://github.com/actix/actix-web/pull/2162
[#2200]: https://github.com/actix/actix-web/pull/2200
[#2201]: https://github.com/actix/actix-web/pull/2201
[#2253]: https://github.com/actix/actix-web/pull/2253
[#2246]: https://github.com/actix/actix-web/pull/2246

## 4.0.0-beta.6

### Added

- `HttpResponse` and `HttpResponseBuilder` types. [#2065]

### Changed

- Most error types are now marked `#[non_exhaustive]`. [#2148]
- Methods on `ContentDisposition` that took `T: AsRef<str>` now take `impl AsRef<str>`.

[#2065]: https://github.com/actix/actix-web/pull/2065
[#2148]: https://github.com/actix/actix-web/pull/2148

## 4.0.0-beta.5

### Added

- `Header` extractor for extracting common HTTP headers in handlers. [#2094]
- Added `TestServer::client_headers` method. [#2097]

### Changed

- `CustomResponder` would return error as `HttpResponse` when `CustomResponder::with_header` failed instead of skipping. (Only the first error is kept when multiple error occur) [#2093]

### Fixed

- Double ampersand in Logger format is escaped correctly. [#2067]

### Removed

- The `client` mod was removed. Clients should now use `awc` directly. [871ca5e4](https://github.com/actix/actix-web/commit/871ca5e4ae2bdc22d1ea02701c2992fa8d04aed7)
- Integration testing was moved to new `actix-test` crate. Namely these items from the `test` module: `TestServer`, `TestServerConfig`, `start`, `start_with`, and `unused_addr`. [#2112]

[#2067]: https://github.com/actix/actix-web/pull/2067
[#2093]: https://github.com/actix/actix-web/pull/2093
[#2094]: https://github.com/actix/actix-web/pull/2094
[#2097]: https://github.com/actix/actix-web/pull/2097
[#2112]: https://github.com/actix/actix-web/pull/2112

## 4.0.0-beta.4

### Changed

- Feature `cookies` is now optional and enabled by default. [#1981]
- `JsonBody::new` returns a default limit of 32kB to be consistent with `JsonConfig` and the default behaviour of the `web::Json<T>` extractor. [#2010]

[#1981]: https://github.com/actix/actix-web/pull/1981
[#2010]: https://github.com/actix/actix-web/pull/2010

## 4.0.0-beta.3

- Update `actix-web-codegen` to `0.5.0-beta.1`.

## 4.0.0-beta.2

### Added

- The method `Either<web::Json<T>, web::Form<T>>::into_inner()` which returns the inner type for whichever variant was created. Also works for `Either<web::Form<T>, web::Json<T>>`. [#1894]
- Add `services!` macro for helping register multiple services to `App`. [#1933]
- Enable registering a vec of services of the same type to `App` [#1933]

### Changed

- Rework `Responder` trait to be sync and returns `Response`/`HttpResponse` directly. Making it simpler and more performant. [#1891]
- `ServiceRequest::into_parts` and `ServiceRequest::from_parts` can no longer fail. [#1893]
- `ServiceRequest::from_request` can no longer fail. [#1893]
- Our `Either` type now uses `Left`/`Right` variants (instead of `A`/`B`) [#1894]
- `test::{call_service, read_response, read_response_json, send_request}` take `&Service` in argument [#1905]
- `App::wrap_fn`, `Resource::wrap_fn` and `Scope::wrap_fn` provide `&Service` in closure argument. [#1905]
- `web::block` no longer requires the output is a Result. [#1957]

### Fixed

- Multiple calls to `App::data` with the same type now keeps the latest call's data. [#1906]

### Removed

- Public field of `web::Path` has been made private. [#1894]
- Public field of `web::Query` has been made private. [#1894]
- `TestRequest::with_header`; use `TestRequest::default().insert_header()`. [#1869]
- `AppService::set_service_data`; for custom HTTP service factories adding application data, use the layered data model by calling `ServiceRequest::add_data_container` when handling requests instead. [#1906]

[#1891]: https://github.com/actix/actix-web/pull/1891
[#1893]: https://github.com/actix/actix-web/pull/1893
[#1894]: https://github.com/actix/actix-web/pull/1894
[#1869]: https://github.com/actix/actix-web/pull/1869
[#1905]: https://github.com/actix/actix-web/pull/1905
[#1906]: https://github.com/actix/actix-web/pull/1906
[#1933]: https://github.com/actix/actix-web/pull/1933
[#1957]: https://github.com/actix/actix-web/pull/1957

## 4.0.0-beta.1

### Added

- `Compat` middleware enabling generic response body/error type of middlewares like `Logger` and `Compress` to be used in `middleware::Condition` and `Resource`, `Scope` services. [#1865]

### Changed

- Update `actix-*` dependencies to tokio `1.0` based versions. [#1813]
- Bumped `rand` to `0.8`.
- Update `rust-tls` to `0.19`. [#1813]
- Rename `Handler` to `HandlerService` and rename `Factory` to `Handler`. [#1852]
- The default `TrailingSlash` is now `Trim`, in line with existing documentation. See migration guide for implications. [#1875]
- Rename `DefaultHeaders::{content_type => add_content_type}`. [#1875]
- MSRV is now 1.46.0.

### Fixed

- Added the underlying parse error to `test::read_body_json`'s panic message. [#1812]

### Removed

- Public modules `middleware::{normalize, err_handlers}`. All necessary middleware types are now exposed directly by the `middleware` module.
- Remove `actix-threadpool` as dependency. `actix_threadpool::BlockingError` error type can be imported from `actix_web::error` module. [#1878]

[#1812]: https://github.com/actix/actix-web/pull/1812
[#1813]: https://github.com/actix/actix-web/pull/1813
[#1852]: https://github.com/actix/actix-web/pull/1852
[#1865]: https://github.com/actix/actix-web/pull/1865
[#1875]: https://github.com/actix/actix-web/pull/1875
[#1878]: https://github.com/actix/actix-web/pull/1878

</details>

## 3.3.3

### Changed

- Soft-deprecate `NormalizePath::default()`, noting upcoming behavior change in v4. [#2529]

[#2529]: https://github.com/actix/actix-web/pull/2529

## 3.3.2

### Fixed

- Removed an occasional `unwrap` on `None` panic in `NormalizePathNormalization`. [#1762]
- Fix `match_pattern()` returning `None` for scope with empty path resource. [#1798]
- Increase minimum `socket2` version. [#1803]

[#1762]: https://github.com/actix/actix-web/pull/1762
[#1798]: https://github.com/actix/actix-web/pull/1798
[#1803]: https://github.com/actix/actix-web/pull/1803

## 3.3.1

- Ensure `actix-http` dependency uses same `serde_urlencoded`.

## 3.3.0

### Added

- Add `Either<A, B>` extractor helper. [#1788]

### Changed

- Upgrade `serde_urlencoded` to `0.7`. [#1773]

[#1773]: https://github.com/actix/actix-web/pull/1773
[#1788]: https://github.com/actix/actix-web/pull/1788

## 3.2.0

### Added

- Implement `exclude_regex` for Logger middleware. [#1723]
- Add request-local data extractor `web::ReqData`. [#1748]
- Add ability to register closure for request middleware logging. [#1749]
- Add `app_data` to `ServiceConfig`. [#1757]
- Expose `on_connect` for access to the connection stream before request is handled. [#1754]

### Changed

- Updated `actix-web-codegen` dependency for access to new `#[route(...)]` multi-method macro.
- Print non-configured `Data<T>` type when attempting extraction. [#1743]
- Re-export `bytes::Buf{Mut}` in web module. [#1750]
- Upgrade `pin-project` to `1.0`.

[#1723]: https://github.com/actix/actix-web/pull/1723
[#1743]: https://github.com/actix/actix-web/pull/1743
[#1748]: https://github.com/actix/actix-web/pull/1748
[#1750]: https://github.com/actix/actix-web/pull/1750
[#1754]: https://github.com/actix/actix-web/pull/1754
[#1757]: https://github.com/actix/actix-web/pull/1757
[#1749]: https://github.com/actix/actix-web/pull/1749

## 3.1.0

### Changed

- Add `TrailingSlash::MergeOnly` behaviour to `NormalizePath`, which allows `NormalizePath` to retain any trailing slashes. [#1695]
- Remove bound `std::marker::Sized` from `web::Data` to support storing `Arc<dyn Trait>` via `web::Data::from` [#1710]

### Fixed

- `ResourceMap` debug printing is no longer infinitely recursive. [#1708]

[#1695]: https://github.com/actix/actix-web/pull/1695
[#1708]: https://github.com/actix/actix-web/pull/1708
[#1710]: https://github.com/actix/actix-web/pull/1710

## 3.0.2

### Fixed

- `NormalizePath` when used with `TrailingSlash::Trim` no longer trims the root path "/". [#1678]

[#1678]: https://github.com/actix/actix-web/pull/1678

## 3.0.1

### Changed

- `middleware::normalize::TrailingSlash` enum is now accessible. [#1673]

[#1673]: https://github.com/actix/actix-web/pull/1673

## 3.0.0

- No significant changes from `3.0.0-beta.4`.

## 3.0.0-beta.4

### Added

- `middleware::NormalizePath` now has configurable behavior for either always having a trailing slash, or as the new addition, always trimming trailing slashes. [#1639]

### Changed

- Update actix-codec and actix-utils dependencies. [#1634]
- `FormConfig` and `JsonConfig` configurations are now also considered when set using `App::data`. [#1641]
- `HttpServer::maxconn` is renamed to the more expressive `HttpServer::max_connections`. [#1655]
- `HttpServer::maxconnrate` is renamed to the more expressive `HttpServer::max_connection_rate`. [#1655]

[#1639]: https://github.com/actix/actix-web/pull/1639
[#1641]: https://github.com/actix/actix-web/pull/1641
[#1634]: https://github.com/actix/actix-web/pull/1634
[#1655]: https://github.com/actix/actix-web/pull/1655

## 3.0.0-beta.3

### Changed

- Update `rustls` to 0.18

## 3.0.0-beta.2

### Changed

- `PayloadConfig` is now also considered in `Bytes` and `String` extractors when set using `App::data`. [#1610]
- `web::Path` now has a public representation: `web::Path(pub T)` that enables destructuring. [#1594]
- `ServiceRequest::app_data` allows retrieval of non-Data data without splitting into parts to access `HttpRequest` which already allows this. [#1618]
- Re-export all error types from `awc`. [#1621]
- MSRV is now 1.42.0.

### Fixed

- Memory leak of app data in pooled requests. [#1609]

[#1594]: https://github.com/actix/actix-web/pull/1594
[#1609]: https://github.com/actix/actix-web/pull/1609
[#1610]: https://github.com/actix/actix-web/pull/1610
[#1618]: https://github.com/actix/actix-web/pull/1618
[#1621]: https://github.com/actix/actix-web/pull/1621

## 3.0.0-beta.1

### Added

- Re-export `actix_rt::main` as `actix_web::main`.
- `HttpRequest::match_pattern` and `ServiceRequest::match_pattern` for extracting the matched resource pattern.
- `HttpRequest::match_name` and `ServiceRequest::match_name` for extracting matched resource name.

### Changed

- Fix actix_http::h1::dispatcher so it returns when HW_BUFFER_SIZE is reached. Should reduce peak memory consumption during large uploads. [#1550]
- Migrate cookie handling to `cookie` crate. Actix-web no longer requires `ring` dependency.
- MSRV is now 1.41.1

### Fixed

- `NormalizePath` improved consistency when path needs slashes added _and_ removed.

## 3.0.0-alpha.3

### Added

- Add option to create `Data<T>` from `Arc<T>` [#1509]

### Changed

- Resources and Scopes can now access non-overridden data types set on App (or containing scopes) when setting their own data. [#1486]
- Fix audit issue logging by default peer address [#1485]
- Bump minimum supported Rust version to 1.40
- Replace deprecated `net2` crate with `socket2`

[#1485]: https://github.com/actix/actix-web/pull/1485
[#1509]: https://github.com/actix/actix-web/pull/1509

## 3.0.0-alpha.2

### Changed

- `{Resource,Scope}::default_service(f)` handlers now support app data extraction. [#1452]
- Implement `std::error::Error` for our custom errors [#1422]
- NormalizePath middleware now appends trailing / so that routes of form /example/ respond to /example requests. [#1433]
- Remove the `failure` feature and support.

[#1422]: https://github.com/actix/actix-web/pull/1422
[#1433]: https://github.com/actix/actix-web/pull/1433
[#1452]: https://github.com/actix/actix-web/pull/1452
[#1486]: https://github.com/actix/actix-web/pull/1486

## 3.0.0-alpha.1

### Added

- Add helper function for creating routes with `TRACE` method guard `web::trace()`
- Add convenience functions `test::read_body_json()` and `test::TestRequest::send_request()` for testing.

### Changed

- Use `sha-1` crate instead of unmaintained `sha1` crate
- Skip empty chunks when returning response from a `Stream` [#1308]
- Update the `time` dependency to 0.2.7
- Update `actix-tls` dependency to 2.0.0-alpha.1
- Update `rustls` dependency to 0.17

[#1308]: https://github.com/actix/actix-web/pull/1308

## 2.0.0

### Changed

- Rename `HttpServer::start()` to `HttpServer::run()`

- Allow to gracefully stop test server via `TestServer::stop()`

- Allow to specify multi-patterns for resources

## 2.0.0-rc

### Changed

- Move `BodyEncoding` to `dev` module #1220

- Allow to set `peer_addr` for TestRequest #1074

- Make web::Data deref to Arc<T> #1214

- Rename `App::register_data()` to `App::app_data()`

- `HttpRequest::app_data<T>()` returns `Option<&T>` instead of `Option<&Data<T>>`

### Fixed

- Fix `AppConfig::secure()` is always false. #1202

## 2.0.0-alpha.6

### Fixed

- Fixed compilation with default features off

## 2.0.0-alpha.5

### Added

- Add test server, `test::start()` and `test::start_with()`

## 2.0.0-alpha.4

### Deleted

- Delete HttpServer::run(), it is not useful with async/await

## 2.0.0-alpha.3

### Changed

- Migrate to tokio 0.2

## 2.0.0-alpha.1

### Changed

- Migrated to `std::future`

- Remove implementation of `Responder` for `()`. (#1167)

## 1.0.9

### Added

- Add `Payload::into_inner` method and make stored `def::Payload` public. (#1110)

### Changed

- Support `Host` guards when the `Host` header is unset (e.g. HTTP/2 requests) (#1129)

## 1.0.8

### Added

- Add `Scope::register_data` and `Resource::register_data` methods, parallel to `App::register_data`.

- Add `middleware::Condition` that conditionally enables another middleware

- Allow to re-construct `ServiceRequest` from `HttpRequest` and `Payload`

- Add `HttpServer::listen_uds` for ability to listen on UDS FD rather than path, which is useful for example with systemd.

### Changed

- Make UrlEncodedError::Overflow more informative

- Use actix-testing for testing utils

## 1.0.7

### Fixed

- Request Extensions leak #1062

## 1.0.6

### Added

- Re-implement Host predicate (#989)

- Form implements Responder, returning a `application/x-www-form-urlencoded` response

- Add `into_inner` to `Data`

- Add `test::TestRequest::set_form()` convenience method to automatically serialize data and set the header in test requests.

### Changed

- `Query` payload made `pub`. Allows user to pattern-match the payload.

- Enable `rust-tls` feature for client #1045

- Update serde_urlencoded to 0.6.1

- Update url to 2.1

## 1.0.5

### Added

- Unix domain sockets (HttpServer::bind_uds) #92

- Actix now logs errors resulting in "internal server error" responses always, with the `error` logging level

### Fixed

- Restored logging of errors through the `Logger` middleware

## 1.0.4

### Added

- Add `Responder` impl for `(T, StatusCode) where T: Responder`

- Allow to access app's resource map via `ServiceRequest::resource_map()` and `HttpRequest::resource_map()` methods.

### Changed

- Upgrade `rand` dependency version to 0.7

## 1.0.3

### Added

- Support asynchronous data factories #850

### Changed

- Use `encoding_rs` crate instead of unmaintained `encoding` crate

## 1.0.2

### Changed

- Move cors middleware to `actix-cors` crate.

- Move identity middleware to `actix-identity` crate.

## 1.0.1

### Added

- Add support for PathConfig #903

- Add `middleware::identity::RequestIdentity` trait to `get_identity` from `HttpMessage`.

### Changed

- Move cors middleware to `actix-cors` crate.

- Move identity middleware to `actix-identity` crate.

- Disable default feature `secure-cookies`.

- Allow to test an app that uses async actors #897

- Re-apply patch from #637 #894

### Fixed

- HttpRequest::url_for is broken with nested scopes #915

## 1.0.0

### Added

- Add `Scope::configure()` method.

- Add `ServiceRequest::set_payload()` method.

- Add `test::TestRequest::set_json()` convenience method to automatically serialize data and set header in test requests.

- Add macros for head, options, trace, connect and patch http methods

### Changed

- Drop an unnecessary `Option<_>` indirection around `ServerBuilder` from `HttpServer`. #863

### Fixed

- Fix Logger request time format, and use rfc3339. #867

- Clear http requests pool on app service drop #860

## 1.0.0-rc

### Added

- Add `Query<T>::from_query()` to extract parameters from a query string. #846
- `QueryConfig`, similar to `JsonConfig` for customizing error handling of query extractors.

### Changed

- `JsonConfig` is now `Send + Sync`, this implies that `error_handler` must be `Send + Sync` too.

### Fixed

- Codegen with parameters in the path only resolves the first registered endpoint #841

## 1.0.0-beta.4

### Added

- Allow to set/override app data on scope level

### Changed

- `App::configure` take an `FnOnce` instead of `Fn`
- Upgrade actix-net crates

## 1.0.0-beta.3

### Added

- Add helper function for executing futures `test::block_fn()`

### Changed

- Extractor configuration could be registered with `App::data()` or with `Resource::data()` #775

- Route data is unified with app data, `Route::data()` moved to resource level to `Resource::data()`

- CORS handling without headers #702

- Allow constructing `Data` instances to avoid double `Arc` for `Send + Sync` types.

### Fixed

- Fix `NormalizePath` middleware impl #806

### Deleted

- `App::data_factory()` is deleted.

## 1.0.0-beta.2

### Added

- Add raw services support via `web::service()`

- Add helper functions for reading response body `test::read_body()`

- Add support for `remainder match` (i.e "/path/{tail}\*")

- Extend `Responder` trait, allow to override status code and headers.

- Store visit and login timestamp in the identity cookie #502

### Changed

- `.to_async()` handler can return `Responder` type #792

### Fixed

- Fix async web::Data factory handling

## 1.0.0-beta.1

### Added

- Add helper functions for reading test response body, `test::read_response()` and test::read_response_json()`

- Add `.peer_addr()` #744

- Add `NormalizePath` middleware

### Changed

- Rename `RouterConfig` to `ServiceConfig`

- Rename `test::call_success` to `test::call_service`

- Removed `ServiceRequest::from_parts()` as it is unsafe to create from parts.

- `CookieIdentityPolicy::max_age()` accepts value in seconds

### Fixed

- Fixed `TestRequest::app_data()`

## 1.0.0-alpha.6

### Changed

- Allow using any service as default service.

- Remove generic type for request payload, always use default.

- Removed `Decompress` middleware. Bytes, String, Json, Form extractors automatically decompress payload.

- Make extractor config type explicit. Add `FromRequest::Config` associated type.

## 1.0.0-alpha.5

### Added

- Added async io `TestBuffer` for testing.

### Deleted

- Removed native-tls support

## 1.0.0-alpha.4

### Added

- `App::configure()` allow to offload app configuration to different methods

- Added `URLPath` option for logger

- Added `ServiceRequest::app_data()`, returns `Data<T>`

- Added `ServiceFromRequest::app_data()`, returns `Data<T>`

### Changed

- `FromRequest` trait refactoring

- Move multipart support to actix-multipart crate

### Fixed

- Fix body propagation in Response::from_error. #760

## 1.0.0-alpha.3

### Changed

- Renamed `TestRequest::to_service()` to `TestRequest::to_srv_request()`

- Renamed `TestRequest::to_response()` to `TestRequest::to_srv_response()`

- Removed `Deref` impls

### Removed

- Removed unused `actix_web::web::md()`

## 1.0.0-alpha.2

### Added

- Rustls support

### Changed

- Use forked cookie

- Multipart::Field renamed to MultipartField

## 1.0.0-alpha.1

### Changed

- Complete architecture re-design.

- Return 405 response if no matching route found within resource #538
