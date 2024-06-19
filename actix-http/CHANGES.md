# Changes

## Unreleased

## 3.8.0

### Added

- Add `error::InvalidStatusCode` re-export.

## 3.7.0

### Added

- Add `rustls-0_23` crate feature
- Add `{h1::H1Service, h2::H2Service, HttpService}::rustls_0_23()` and `HttpService::rustls_0_23_with_config()` service constructors.

### Changed

- Update `brotli` dependency to `6`.
- Minimum supported Rust version (MSRV) is now 1.72.

## 3.6.0

### Added

- Add `rustls-0_22` crate feature.
- Add `{h1::H1Service, h2::H2Service, HttpService}::rustls_0_22()` and `HttpService::rustls_0_22_with_config()` service constructors.
- Implement `From<&HeaderMap>` for `http::HeaderMap`.

## 3.5.1

### Fixed

- Prevent hang when returning zero-sized response bodies through compression layer.

## 3.5.0

### Added

- Implement `From<HeaderMap>` for `http::HeaderMap`.

### Changed

- Updated `zstd` dependency to `0.13`.

### Fixed

- Prevent compression of zero-sized response bodies.

## 3.4.0

### Added

- Add `rustls-0_20` crate feature.
- Add `{h1::H1Service, h2::H2Service, HttpService}::rustls_021()` and `HttpService::rustls_021_with_config()` service constructors.
- Add `body::to_bytes_limited()` function.
- Add `body::BodyLimitExceeded` error type.

### Changed

- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 3.3.1

### Fixed

- Use correct `http` version requirement to ensure support for const `HeaderName` definitions.

## 3.3.0

### Added

- Implement `MessageBody` for `Cow<'static, str>` and `Cow<'static, [u8]>`. [#2959]
- Implement `MessageBody` for `&mut B` where `B: MessageBody + Unpin`. [#2868]
- Implement `MessageBody` for `Pin<B>` where `B::Target: MessageBody`. [#2868]
- Automatic h2c detection via new service finalizer `HttpService::tcp_auto_h2c()`. [#2957]
- `HeaderMap::retain()`. [#2955]
- Header name constants in `header` module. [#2956] [#2968]
  - `CACHE_STATUS`
  - `CDN_CACHE_CONTROL`
  - `CROSS_ORIGIN_EMBEDDER_POLICY`
  - `CROSS_ORIGIN_OPENER_POLICY`
  - `PERMISSIONS_POLICY`
  - `X_FORWARDED_FOR`
  - `X_FORWARDED_HOST`
  - `X_FORWARDED_PROTO`

### Fixed

- Fix non-empty body of HTTP/2 HEAD responses. [#2920]

### Performance

- Improve overall performance of operations on `Extensions`. [#2890]

[#2959]: https://github.com/actix/actix-web/pull/2959
[#2868]: https://github.com/actix/actix-web/pull/2868
[#2890]: https://github.com/actix/actix-web/pull/2890
[#2920]: https://github.com/actix/actix-web/pull/2920
[#2957]: https://github.com/actix/actix-web/pull/2957
[#2955]: https://github.com/actix/actix-web/pull/2955
[#2956]: https://github.com/actix/actix-web/pull/2956
[#2968]: https://github.com/actix/actix-web/pull/2968

## 3.2.2

### Changed

- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

### Fixed

- Avoid possibility of dispatcher getting stuck while back-pressuring I/O. [#2369]

[#2369]: https://github.com/actix/actix-web/pull/2369

## 3.2.1

### Fixed

- Fix parsing ambiguity in Transfer-Encoding and Content-Length headers for HTTP/1.0 requests. [#2794]

[#2794]: https://github.com/actix/actix-web/pull/2794

## 3.2.0

### Changed

- Minimum supported Rust version (MSRV) is now 1.57 due to transitive `time` dependency.

### Fixed

- Websocket parser no longer throws endless overflow errors after receiving an oversized frame. [#2790]
- Retain previously set Vary headers when using compression encoder. [#2798]

[#2790]: https://github.com/actix/actix-web/pull/2790
[#2798]: https://github.com/actix/actix-web/pull/2798

## 3.1.0

### Changed

- Minimum supported Rust version (MSRV) is now 1.56 due to transitive `hashbrown` dependency.

### Fixed

- Revert broken fix in [#2624] that caused erroneous 500 error responses. Temporarily re-introduces [#2357] bug. [#2779]

[#2624]: https://github.com/actix/actix-web/pull/2624
[#2357]: https://github.com/actix/actix-web/issues/2357
[#2779]: https://github.com/actix/actix-web/pull/2779

## 3.0.4

### Fixed

- Document on docs.rs with `ws` feature enabled.

## 3.0.3

### Fixed

- Allow spaces between header name and colon when parsing responses. [#2684]

[#2684]: https://github.com/actix/actix-web/pull/2684

## 3.0.2

### Fixed

- Fix encoding camel-case header names with more than one hyphen. [#2683]

[#2683]: https://github.com/actix/actix-web/pull/2683

## 3.0.1

- Fix panic in H1 dispatcher when pipelining is used with keep-alive. [#2678]

[#2678]: https://github.com/actix/actix-web/issues/2678

## 3.0.0

### Dependencies

- Updated `actix-*` to Tokio v1-based versions. [#1813]
- Updated `bytes` to `1.0`. [#1813]
- Updated `h2` to `0.3`. [#1813]
- Updated `rustls` to `0.20.0`. [#2414]
- Updated `language-tags` to `0.3`.
- Updated `tokio` to `1`.

### Added

- Crate Features:
  - `ws`; disabled by default. [#2618]
  - `http2`; disabled by default. [#2618]
  - `compress-brotli`; disabled by default. [#2618]
  - `compress-gzip`; disabled by default. [#2618]
  - `compress-zstd`; disabled by default. [#2618]
- Functions:
  - `body::to_bytes` for async collecting message body into Bytes. [#2158]
- Traits:
  - `TryIntoHeaderPair`; allows using typed and untyped headers in the same methods. [#1869]
- Types:
  - `body::BoxBody`; a boxed message body with boxed errors. [#2183]
  - `body::EitherBody` enum. [#2468]
  - `body::None` struct. [#2468]
  - Re-export `http` crate's `Error` type as `error::HttpError`. [#2171]
- Variants:
  - `ContentEncoding::Zstd` along with . [#2244]
  - `Protocol::Http3` for future compatibility and also mark `#[non_exhaustive]`. [00ba8d55]
- Methods:
  - `ContentEncoding::to_header_value()`. [#2501]
  - `header::QualityItem::{max, min}()`. [#2486]
  - `header::QualityItem::zero()` that uses `Quality::ZERO`. [#2501]
  - `HeaderMap::drain()` as an efficient draining iterator. [#1964]
  - `HeaderMap::len_keys()` has the behavior of the old `len` method. [#1964]
  - `MessageBody::boxed` trait method for wrapping boxing types efficiently. [#2520]
  - `MessageBody::try_into_bytes` trait method, with default implementation, for optimizations on body types that complete in exactly one poll. [#2522]
  - `Request::conn_data()`. [#2491]
  - `Request::take_conn_data()`. [#2491]
  - `Request::take_req_data()`. [#2487]
  - `Response::{ok, bad_request, not_found, internal_server_error}()`. [#2159]
  - `Response::into_body()` that consumes response and returns body type. [#2201]
  - `Response::map_into_boxed_body()`. [#2468]
  - `ResponseBuilder::append_header()` method which allows using typed and untyped headers. [#1869]
  - `ResponseBuilder::insert_header()` method which allows using typed and untyped headers. [#1869]
  - `ResponseHead::set_camel_case_headers()`. [#2587]
  - `TestRequest::insert_header()` method which allows using typed and untyped headers. [#1869]
- Implementations:
  - Implement `Clone for ws::HandshakeError`. [#2468]
  - Implement `Clone` for `body::AnyBody<S> where S: Clone`. [#2448]
  - Implement `Clone` for `RequestHead`. [#2487]
  - Implement `Clone` for `ResponseHead`. [#2585]
  - Implement `Copy` for `QualityItem<T> where T: Copy`. [#2501]
  - Implement `Default` for `ContentEncoding`. [#1912]
  - Implement `Default` for `HttpServiceBuilder`. [#2611]
  - Implement `Default` for `KeepAlive`. [#2611]
  - Implement `Default` for `Response`. [#2201]
  - Implement `Default` for `ws::Codec`. [#1920]
  - Implement `Display` for `header::Quality`. [#2486]
  - Implement `Eq` for `header::ContentEncoding`. [#2501]
  - Implement `ExactSizeIterator` and `FusedIterator` for all `HeaderMap` iterators. [#2470]
  - Implement `From<Duration>` for `KeepAlive`. [#2611]
  - Implement `From<Option<Duration>>` for `KeepAlive`. [#2611]
  - Implement `From<Vec<u8>>` for `Response<Vec<u8>>`. [#2625]
  - Implement `FromStr` for `ContentEncoding`. [#1912]
  - Implement `Header` for `ContentEncoding`. [#1912]
  - Implement `IntoHeaderValue` for `ContentEncoding`. [#1912]
  - Implement `IntoIterator` for `HeaderMap`. [#1964]
  - Implement `MessageBody` for `bytestring::ByteString`. [#2468]
  - Implement `MessageBody` for `Pin<Box<T>> where T: MessageBody`. [#2152]
- Misc:
  - Re-export `StatusCode`, `Method`, `Version` and `Uri` at the crate root. [#2171]
  - Re-export `ContentEncoding` and `ConnectionType` at the crate root. [#2171]
  - `Quality::ZERO` associated constant equivalent to `q=0`. [#2501]
  - `header::Quality::{MAX, MIN}` associated constants equivalent to `q=1` and `q=0.001`, respectively. [#2486]
  - Timeout for canceling HTTP/2 server side connection handshake. Configurable with `ServiceConfig::client_timeout`; defaults to 5 seconds. [#2483]
  - `#[must_use]` for `ws::Codec` to prevent subtle bugs. [#1920]

### Changed

- Traits:
  - Rename `IntoHeaderValue => TryIntoHeaderValue`. [#2510]
  - `MessageBody` now has an associated `Error` type. [#2183]
- Types:
  - `Protocol` enum is now marked `#[non_exhaustive]`.
  - `error::DispatcherError` enum is now marked `#[non_exhaustive]`. [#2624]
  - `ContentEncoding` is now marked `#[non_exhaustive]`. [#2377]
  - Error enums are marked `#[non_exhaustive]`. [#2161]
  - Rename `PayloadStream` to `BoxedPayloadStream`. [#2545]
  - The body type parameter of `Response` no longer has a default. [#2152]
- Enum Variants:
  - Rename `ContentEncoding::{Br => Brotli}`. [#2501]
  - `Payload` inner fields are now named. [#2545]
  - `ws::Message::Text` now contains a `bytestring::ByteString`. [#1864]
- Methods:
  - Rename `ServiceConfig::{client_timer_expire => client_request_deadline}`. [#2611]
  - Rename `ServiceConfig::{client_disconnect_timer => client_disconnect_deadline}`. [#2611]
  - Rename `h1::Codec::{keepalive => keep_alive}`. [#2611]
  - Rename `h1::Codec::{keepalive_enabled => keep_alive_enabled}`. [#2611]
  - Rename `h1::ClientCodec::{keepalive => keep_alive}`. [#2611]
  - Rename `h1::ClientPayloadCodec::{keepalive => keep_alive}`. [#2611]
  - Rename `header::EntityTag::{weak => new_weak, strong => new_strong}`. [#2565]
  - Rename `TryIntoHeaderValue::{try_into => try_into_value}` to avoid ambiguity with std `TryInto` trait. [#1894]
  - Deadline methods in `ServiceConfig` now return `std::time::Instant`s instead of Tokio's wrapper type. [#2611]
  - Places in `Response` where `ResponseBody<B>` was received or returned now simply use `B`. [#2201]
  - `encoding::Encoder::response` now returns `AnyBody<Encoder<B>>`. [#2448]
  - `Extensions::insert` returns replaced item. [#1904]
  - `HeaderMap::get_all` now returns a `std::slice::Iter`. [#2527]
  - `HeaderMap::insert` now returns iterator of removed values. [#1964]
  - `HeaderMap::len` now returns number of values instead of number of keys. [#1964]
  - `HeaderMap::remove` now returns iterator of removed values. [#1964]
  - `ResponseBuilder::body(B)` now returns `Response<EitherBody<B>>`. [#2468]
  - `ResponseBuilder::content_type` now takes an `impl TryIntoHeaderValue` to support using typed `mime` types. [#1894]
  - `ResponseBuilder::finish()` now returns `Response<EitherBody<()>>`. [#2468]
  - `ResponseBuilder::json` now takes `impl Serialize`. [#2052]
  - `ResponseBuilder::message_body` now returns a `Result`. [#2201]∑
  - `ServiceConfig::keep_alive` now returns a `KeepAlive`. [#2611]
  - `ws::hash_key` now returns array. [#2035]
- Trait Implementations:
  - Implementation of `Stream` for `Payload` no longer requires the `Stream` variant be `Unpin`. [#2545]
  - Implementation of `Future` for `h1::SendResponse` no longer requires the body type be `Unpin`. [#2545]
  - Implementation of `Stream` for `encoding::Decoder` no longer requires the stream type be `Unpin`. [#2545]
  - Implementation of `From` for error types now return a `Response<BoxBody>`. [#2468]
- Misc:
  - `header` module is now public. [#2171]
  - `uri` module is now public. [#2171]
  - Request-local data container is no longer part of a `RequestHead`. Instead it is a distinct part of a `Request`. [#2487]
  - All error trait bounds in server service builders have changed from `Into<Error>` to `Into<Response<BoxBody>>`. [#2253]
  - All error trait bounds in message body and stream impls changed from `Into<Error>` to `Into<Box<dyn std::error::Error>>`. [#2253]
  - Guarantee ordering of `header::GetAll` iterator to be same as insertion order. [#2467]
  - Connection data set through the `on_connect_ext` callbacks is now accessible only from the new `Request::conn_data()` method. [#2491]
  - Brotli (de)compression support is now provided by the `brotli` crate. [#2538]
  - Minimum supported Rust version (MSRV) is now 1.54.

### Fixed

- A `Vary` header is now correctly sent along with compressed content. [#2501]
- HTTP/1.1 dispatcher correctly uses client request timeout. [#2611]
- Fixed issue where handlers that took payload but then dropped without reading it to EOF it would cause keep-alive connections to become stuck. [#2624]
- `ContentEncoding`'s `Identity` variant can now be parsed from a string. [#2501]
- `HttpServer::{listen_rustls(), bind_rustls()}` now honor the ALPN protocols in the configuration parameter. [#2226]
- Remove unnecessary `Into<Error>` bound on `Encoder` body types. [#2375]
- Remove unnecessary `Unpin` bound on `ResponseBuilder::streaming`. [#2253]
- `BodyStream` and `SizedStream` are no longer restricted to `Unpin` types. [#2152]
- Fixed slice creation pointing to potential uninitialized data on h1 encoder. [#2364]
- Fixed quality parse error in Accept-Encoding header. [#2344]

### Removed

- Crate Features:
  - `compress` feature. [#2065]
  - `cookies` feature. [#2065]
  - `trust-dns` feature. [#2425]
  - `actors` optional feature and trait implementation for `actix` types. [#1969]
- Functions:
  - `header::qitem` helper. Replaced with `header::QualityItem::max`. [#2486]
- Types:
  - `body::Body`; replaced with `EitherBody` and `BoxBody`. [#2468]
  - `body::ResponseBody`. [#2446]
  - `ConnectError::SslHandshakeError` and re-export of `HandshakeError`. Due to the removal of this type from `tokio-openssl` crate. OpenSSL handshake error now returns `ConnectError::SslError`. [#1813]
  - `error::Canceled` re-export. [#1994]
  - `error::Result` type alias. [#2201]
  - `error::BlockingError` [#2660]
  - `InternalError` and all the error types it constructed were moved up to `actix-web`. [#2215]
  - Typed HTTP headers; they have moved up to `actix-web`. [2094]
  - Re-export of `http` crate's `HeaderMap` types in addition to ours. [#2171]
- Enum Variants:
  - `body::BodySize::Empty`; an empty body can now only be represented as a `Sized(0)` variant. [#2446]
  - `ContentEncoding::Auto`. [#2501]
  - `EncoderError::Boxed`. [#2446]
- Methods:
  - `ContentEncoding::is_compression()`. [#2501]
  - `h1::Payload::readany()`. [#2545]
  - `HttpMessage::cookie[s]()` trait methods. [#2065]
  - `HttpServiceBuilder::new()`; use `default` instead. [#2611]
  - `on_connect` (previously deprecated) methods have been removed; use `on_connect_ext`. [#1857]
  - `Response::build_from()`. [#2159]
  - `Response::error()` [#2205]
  - `Response::take_body()` and old `Response::into_body()` method that casted body type. [#2201]
  - `Response`'s status code builders. [#2159]
  - `ResponseBuilder::{if_true, if_some}()` (previously deprecated). [#2148]
  - `ResponseBuilder::{set, set_header}()`; use `ResponseBuilder::insert_header()`. [#1869]
  - `ResponseBuilder::extensions[_mut]()`. [#2585]
  - `ResponseBuilder::header()`; use `ResponseBuilder::append_header()`. [#1869]
  - `ResponseBuilder::json()`. [#2148]
  - `ResponseBuilder::json2()`. [#1903]
  - `ResponseBuilder::streaming()`. [#2468]
  - `ResponseHead::extensions[_mut]()`. [#2585]
  - `ServiceConfig::{client_timer, keep_alive_timer}()`. [#2611]
  - `TestRequest::with_hdr()`; use `TestRequest::default().insert_header()`. [#1869]
  - `TestRequest::with_header()`; use `TestRequest::default().insert_header()`. [#1869]
- Trait implementations:
  - Implementation of `Copy` for `ws::Codec`. [#1920]
  - Implementation of `From<Option<usize>> for KeepAlive`; use `Duration`s instead. [#2611]
  - Implementation of `From<serde_json::Value>` for `Body`. [#2148]
  - Implementation of `From<usize> for KeepAlive`; use `Duration`s instead. [#2611]
  - Implementation of `Future` for `Response`. [#2201]
  - Implementation of `Future` for `ResponseBuilder`. [#2468]
  - Implementation of `Into<Error>` for `Response<Body>`. [#2215]
  - Implementation of `Into<Error>` for `ResponseBuilder`. [#2215]
  - Implementation of `ResponseError` for `actix_utils::timeout::TimeoutError`. [#2127]
  - Implementation of `ResponseError` for `CookieParseError`. [#2065]
  - Implementation of `TryFrom<u16>` for `header::Quality`. [#2486]
- Misc:
  - `http` module; most everything it contained is exported at the crate root. [#2488]
  - `cookies` module (re-export). [#2065]
  - `client` module. Connector types now live in `awc`. [#2425]
  - `error` field from `Response`. [#2205]
  - `downcast` and `downcast_get_type_id` macros. [#2291]
  - Down-casting for `MessageBody` types; use standard `Any` trait. [#2183]

[#1813]: https://github.com/actix/actix-web/pull/1813
[#1845]: https://github.com/actix/actix-web/pull/1845
[#1857]: https://github.com/actix/actix-web/pull/1857
[#1864]: https://github.com/actix/actix-web/pull/1864
[#1869]: https://github.com/actix/actix-web/pull/1869
[#1878]: https://github.com/actix/actix-web/pull/1878
[#1894]: https://github.com/actix/actix-web/pull/1894
[#1903]: https://github.com/actix/actix-web/pull/1903
[#1904]: https://github.com/actix/actix-web/pull/1904
[#1912]: https://github.com/actix/actix-web/pull/1912
[#1920]: https://github.com/actix/actix-web/pull/1920
[#1964]: https://github.com/actix/actix-web/pull/1964
[#1969]: https://github.com/actix/actix-web/pull/1969
[#1981]: https://github.com/actix/actix-web/pull/1981
[#1994]: https://github.com/actix/actix-web/pull/1994
[#2035]: https://github.com/actix/actix-web/pull/2035
[#2052]: https://github.com/actix/actix-web/pull/2052
[#2065]: https://github.com/actix/actix-web/pull/2065
[#2094]: https://github.com/actix/actix-web/pull/2094
[#2127]: https://github.com/actix/actix-web/pull/2127
[#2148]: https://github.com/actix/actix-web/pull/2148
[#2152]: https://github.com/actix/actix-web/pull/2152
[#2158]: https://github.com/actix/actix-web/pull/2158
[#2159]: https://github.com/actix/actix-web/pull/2159
[#2161]: https://github.com/actix/actix-web/pull/2161
[#2171]: https://github.com/actix/actix-web/pull/2171
[#2183]: https://github.com/actix/actix-web/pull/2183
[#2196]: https://github.com/actix/actix-web/pull/2196
[#2201]: https://github.com/actix/actix-web/pull/2201
[#2205]: https://github.com/actix/actix-web/pull/2205
[#2215]: https://github.com/actix/actix-web/pull/2215
[#2244]: https://github.com/actix/actix-web/pull/2244
[#2250]: https://github.com/actix/actix-web/pull/2250
[#2253]: https://github.com/actix/actix-web/pull/2253
[#2291]: https://github.com/actix/actix-web/pull/2291
[#2344]: https://github.com/actix/actix-web/pull/2344
[#2364]: https://github.com/actix/actix-web/pull/2364
[#2375]: https://github.com/actix/actix-web/pull/2375
[#2377]: https://github.com/actix/actix-web/pull/2377
[#2414]: https://github.com/actix/actix-web/pull/2414
[#2425]: https://github.com/actix/actix-web/pull/2425
[#2442]: https://github.com/actix/actix-web/pull/2442
[#2446]: https://github.com/actix/actix-web/pull/2446
[#2448]: https://github.com/actix/actix-web/pull/2448
[#2456]: https://github.com/actix/actix-web/pull/2456
[#2467]: https://github.com/actix/actix-web/pull/2467
[#2468]: https://github.com/actix/actix-web/pull/2468
[#2470]: https://github.com/actix/actix-web/pull/2470
[#2474]: https://github.com/actix/actix-web/pull/2474
[#2483]: https://github.com/actix/actix-web/pull/2483
[#2486]: https://github.com/actix/actix-web/pull/2486
[#2487]: https://github.com/actix/actix-web/pull/2487
[#2488]: https://github.com/actix/actix-web/pull/2488
[#2491]: https://github.com/actix/actix-web/pull/2491
[#2497]: https://github.com/actix/actix-web/pull/2497
[#2501]: https://github.com/actix/actix-web/pull/2501
[#2510]: https://github.com/actix/actix-web/pull/2510
[#2520]: https://github.com/actix/actix-web/pull/2520
[#2522]: https://github.com/actix/actix-web/pull/2522
[#2527]: https://github.com/actix/actix-web/pull/2527
[#2538]: https://github.com/actix/actix-web/pull/2538
[#2545]: https://github.com/actix/actix-web/pull/2545
[#2565]: https://github.com/actix/actix-web/pull/2565
[#2585]: https://github.com/actix/actix-web/pull/2585
[#2587]: https://github.com/actix/actix-web/pull/2587
[#2611]: https://github.com/actix/actix-web/pull/2611
[#2618]: https://github.com/actix/actix-web/pull/2618
[#2624]: https://github.com/actix/actix-web/pull/2624
[#2625]: https://github.com/actix/actix-web/pull/2625
[#2660]: https://github.com/actix/actix-web/pull/2660
[00ba8d55]: https://github.com/actix/actix-web/commit/00ba8d55492284581695d824648590715a8bd386

<details>
<summary>3.0.0 Pre-Releases</summary>

## 3.0.0-rc.4

### Fixed

- Fix h1 dispatcher panic. [1ce58ecb]

[1ce58ecb]: https://github.com/actix/actix-web/commit/1ce58ecb305c60e51db06e6c913b7a1344e229ca

## 3.0.0-rc.3

- No significant changes since `3.0.0-rc.2`.

## 3.0.0-rc.2

### Added

- Implement `From<Vec<u8>>` for `Response<Vec<u8>>`. [#2625]

### Changed

- `error::DispatcherError` enum is now marked `#[non_exhaustive]`. [#2624]

### Fixed

- Issue where handlers that took payload but then dropped without reading it to EOF it would cause keep-alive connections to become stuck. [#2624]

[#2624]: https://github.com/actix/actix-web/pull/2624
[#2625]: https://github.com/actix/actix-web/pull/2625

## 3.0.0-rc.1

### Added

- Implement `Default` for `KeepAlive`. [#2611]
- Implement `From<Duration>` for `KeepAlive`. [#2611]
- Implement `From<Option<Duration>>` for `KeepAlive`. [#2611]
- Implement `Default` for `HttpServiceBuilder`. [#2611]
- Crate `ws` feature flag, disabled by default. [#2618]
- Crate `http2` feature flag, disabled by default. [#2618]

### Changed

- Rename `ServiceConfig::{client_timer_expire => client_request_deadline}`. [#2611]
- Rename `ServiceConfig::{client_disconnect_timer => client_disconnect_deadline}`. [#2611]
- Deadline methods in `ServiceConfig` now return `std::time::Instant`s instead of Tokio's wrapper type. [#2611]
- Rename `h1::Codec::{keepalive => keep_alive}`. [#2611]
- Rename `h1::Codec::{keepalive_enabled => keep_alive_enabled}`. [#2611]
- Rename `h1::ClientCodec::{keepalive => keep_alive}`. [#2611]
- Rename `h1::ClientPayloadCodec::{keepalive => keep_alive}`. [#2611]
- `ServiceConfig::keep_alive` now returns a `KeepAlive`. [#2611]

### Fixed

- HTTP/1.1 dispatcher correctly uses client request timeout. [#2611]

### Removed

- `ServiceConfig::{client_timer, keep_alive_timer}`. [#2611]
- `impl From<usize> for KeepAlive`; use `Duration`s instead. [#2611]
- `impl From<Option<usize>> for KeepAlive`; use `Duration`s instead. [#2611]
- `HttpServiceBuilder::new`; use `default` instead. [#2611]

[#2611]: https://github.com/actix/actix-web/pull/2611
[#2618]: https://github.com/actix/actix-web/pull/2618

## 3.0.0-beta.19

### Added

- Response headers can be sent as camel case using `res.head_mut().set_camel_case_headers(true)`. [#2587]
- `ResponseHead` now implements `Clone`. [#2585]

### Changed

- Brotli (de)compression support is now provided by the `brotli` crate. [#2538]

### Removed

- `ResponseHead::extensions[_mut]()`. [#2585]
- `ResponseBuilder::extensions[_mut]()`. [#2585]

[#2538]: https://github.com/actix/actix-web/pull/2538
[#2585]: https://github.com/actix/actix-web/pull/2585
[#2587]: https://github.com/actix/actix-web/pull/2587

## 3.0.0-beta.18

### Added

- `impl Eq` for `header::ContentEncoding`. [#2501]
- `impl Copy` for `QualityItem` where `T: Copy`. [#2501]
- `Quality::ZERO` equivalent to `q=0`. [#2501]
- `QualityItem::zero` that uses `Quality::ZERO`. [#2501]
- `ContentEncoding::to_header_value()`. [#2501]

### Changed

- `Quality::MIN` is now the smallest non-zero value. [#2501]
- `QualityItem::min` semantics changed with `QualityItem::MIN`. [#2501]
- Rename `ContentEncoding::{Br => Brotli}`. [#2501]
- Rename `header::EntityTag::{weak => new_weak, strong => new_strong}`. [#2565]
- Minimum supported Rust version (MSRV) is now 1.54.

### Fixed

- `ContentEncoding::Identity` can now be parsed from a string. [#2501]
- A `Vary` header is now correctly sent along with compressed content. [#2501]

### Removed

- `ContentEncoding::Auto` variant. [#2501]
- `ContentEncoding::is_compression()`. [#2501]

[#2501]: https://github.com/actix/actix-web/pull/2501
[#2565]: https://github.com/actix/actix-web/pull/2565

## 3.0.0-beta.17

### Changed

- `HeaderMap::get_all` now returns a `std::slice::Iter`. [#2527]
- `Payload` inner fields are now named. [#2545]
- `impl Stream` for `Payload` no longer requires the `Stream` variant be `Unpin`. [#2545]
- `impl Future` for `h1::SendResponse` no longer requires the body type be `Unpin`. [#2545]
- `impl Stream` for `encoding::Decoder` no longer requires the stream type be `Unpin`. [#2545]
- Rename `PayloadStream` to `BoxedPayloadStream`. [#2545]

### Removed

- `h1::Payload::readany`. [#2545]

[#2527]: https://github.com/actix/actix-web/pull/2527
[#2545]: https://github.com/actix/actix-web/pull/2545

## 3.0.0-beta.16

### Added

- New method on `MessageBody` trait, `try_into_bytes`, with default implementation, for optimizations on body types that complete in exactly one poll. Replaces `is_complete_body` and `take_complete_body`. [#2522]

### Changed

- Rename trait `IntoHeaderPair => TryIntoHeaderPair`. [#2510]
- Rename `TryIntoHeaderPair::{try_into_header_pair => try_into_pair}`. [#2510]
- Rename trait `IntoHeaderValue => TryIntoHeaderValue`. [#2510]

### Removed

- `MessageBody::{is_complete_body,take_complete_body}`. [#2522]

[#2510]: https://github.com/actix/actix-web/pull/2510
[#2522]: https://github.com/actix/actix-web/pull/2522

## 3.0.0-beta.15

### Added

- Add timeout for canceling HTTP/2 server side connection handshake. Default to 5 seconds. [#2483]
- HTTP/2 handshake timeout can be configured with `ServiceConfig::client_timeout`. [#2483]
- `Response::map_into_boxed_body`. [#2468]
- `body::EitherBody` enum. [#2468]
- `body::None` struct. [#2468]
- Impl `MessageBody` for `bytestring::ByteString`. [#2468]
- `impl Clone for ws::HandshakeError`. [#2468]
- `#[must_use]` for `ws::Codec` to prevent subtle bugs. [#1920]
- `impl Default ` for `ws::Codec`. [#1920]
- `header::QualityItem::{max, min}`. [#2486]
- `header::Quality::{MAX, MIN}`. [#2486]
- `impl Display` for `header::Quality`. [#2486]
- Connection data set through the `on_connect_ext` callbacks is now accessible only from the new `Request::conn_data()` method. [#2491]
- `Request::take_conn_data()`. [#2491]
- `Request::take_req_data()`. [#2487]
- `impl Clone` for `RequestHead`. [#2487]
- New methods on `MessageBody` trait, `is_complete_body` and `take_complete_body`, both with default implementations, for optimizations on body types that are done in exactly one poll/chunk. [#2497]
- New `boxed` method on `MessageBody` trait for wrapping body type. [#2520]

### Changed

- Rename `body::BoxBody::{from_body => new}`. [#2468]
- Body type for `Responses` returned from `Response::{new, ok, etc...}` is now `BoxBody`. [#2468]
- The `Error` associated type on `MessageBody` type now requires `impl Error` (or similar). [#2468]
- Error types using in service builders now require `Into<Response<BoxBody>>`. [#2468]
- `From` implementations on error types now return a `Response<BoxBody>`. [#2468]
- `ResponseBuilder::body(B)` now returns `Response<EitherBody<B>>`. [#2468]
- `ResponseBuilder::finish()` now returns `Response<EitherBody<()>>`. [#2468]

### Removed

- `ResponseBuilder::streaming`. [#2468]
- `impl Future` for `ResponseBuilder`. [#2468]
- Remove unnecessary `MessageBody` bound on types passed to `body::AnyBody::new`. [#2468]
- Move `body::AnyBody` to `awc`. Replaced with `EitherBody` and `BoxBody`. [#2468]
- `impl Copy` for `ws::Codec`. [#1920]
- `header::qitem` helper. Replaced with `header::QualityItem::max`. [#2486]
- `impl TryFrom<u16>` for `header::Quality`. [#2486]
- `http` module. Most everything it contained is exported at the crate root. [#2488]

[#2483]: https://github.com/actix/actix-web/pull/2483
[#2468]: https://github.com/actix/actix-web/pull/2468
[#1920]: https://github.com/actix/actix-web/pull/1920
[#2486]: https://github.com/actix/actix-web/pull/2486
[#2487]: https://github.com/actix/actix-web/pull/2487
[#2488]: https://github.com/actix/actix-web/pull/2488
[#2491]: https://github.com/actix/actix-web/pull/2491
[#2497]: https://github.com/actix/actix-web/pull/2497
[#2520]: https://github.com/actix/actix-web/pull/2520

## 3.0.0-beta.14

### Changed

- Guarantee ordering of `header::GetAll` iterator to be same as insertion order. [#2467]
- Expose `header::map` module. [#2467]
- Implement `ExactSizeIterator` and `FusedIterator` for all `HeaderMap` iterators. [#2470]
- Update `actix-tls` to `3.0.0-rc.1`. [#2474]

[#2467]: https://github.com/actix/actix-web/pull/2467
[#2470]: https://github.com/actix/actix-web/pull/2470
[#2474]: https://github.com/actix/actix-web/pull/2474

## 3.0.0-beta.13

### Added

- `body::AnyBody::empty` for quickly creating an empty body. [#2446]
- `body::AnyBody::none` for quickly creating a "none" body. [#2456]
- `impl Clone` for `body::AnyBody<S> where S: Clone`. [#2448]
- `body::AnyBody::into_boxed` for quickly converting to a type-erased, boxed body type. [#2448]

### Changed

- Rename `body::AnyBody::{Message => Body}`. [#2446]
- Rename `body::AnyBody::{from_message => new_boxed}`. [#2448]
- Rename `body::AnyBody::{from_slice => copy_from_slice}`. [#2448]
- Rename `body::{BoxAnyBody => BoxBody}`. [#2448]
- Change representation of `AnyBody` to include a type parameter in `Body` variant. Defaults to `BoxBody`. [#2448]
- `Encoder::response` now returns `AnyBody<Encoder<B>>`. [#2448]

### Removed

- `body::AnyBody::Empty`; an empty body can now only be represented as a zero-length `Bytes` variant. [#2446]
- `body::BodySize::Empty`; an empty body can now only be represented as a `Sized(0)` variant. [#2446]
- `EncoderError::Boxed`; it is no longer required. [#2446]
- `body::ResponseBody`; is function is replaced by the new `body::AnyBody` enum. [#2446]

[#2446]: https://github.com/actix/actix-web/pull/2446
[#2448]: https://github.com/actix/actix-web/pull/2448
[#2456]: https://github.com/actix/actix-web/pull/2456

## 3.0.0-beta.12

### Changed

- Update `actix-server` to `2.0.0-beta.9`. [#2442]

### Removed

- `client` module. [#2425]
- `trust-dns` feature. [#2425]

[#2425]: https://github.com/actix/actix-web/pull/2425
[#2442]: https://github.com/actix/actix-web/pull/2442

## 3.0.0-beta.11

### Changed

- Updated rustls to v0.20. [#2414]
- Minimum supported Rust version (MSRV) is now 1.52.

[#2414]: https://github.com/actix/actix-web/pull/2414

## 3.0.0-beta.10

### Changed

- `ContentEncoding` is now marked `#[non_exhaustive]`. [#2377]
- Minimum supported Rust version (MSRV) is now 1.51.

### Fixed

- Remove slice creation pointing to potential uninitialized data on h1 encoder. [#2364]
- Remove `Into<Error>` bound on `Encoder` body types. [#2375]
- Fix quality parse error in Accept-Encoding header. [#2344]

[#2364]: https://github.com/actix/actix-web/pull/2364
[#2375]: https://github.com/actix/actix-web/pull/2375
[#2344]: https://github.com/actix/actix-web/pull/2344
[#2377]: https://github.com/actix/actix-web/pull/2377

## 3.0.0-beta.9

### Fixed

- Potential HTTP request smuggling vulnerabilities. [RUSTSEC-2021-0081](https://github.com/rustsec/advisory-db/pull/977)

## 3.0.0-beta.8

### Changed

- Change compression algorithm features flags. [#2250]

### Removed

- `downcast` and `downcast_get_type_id` macros. [#2291]

[#2291]: https://github.com/actix/actix-web/pull/2291
[#2250]: https://github.com/actix/actix-web/pull/2250

## 3.0.0-beta.7

### Added

- Alias `body::Body` as `body::AnyBody`. [#2215]
- `BoxAnyBody`: a boxed message body with boxed errors. [#2183]
- Re-export `http` crate's `Error` type as `error::HttpError`. [#2171]
- Re-export `StatusCode`, `Method`, `Version` and `Uri` at the crate root. [#2171]
- Re-export `ContentEncoding` and `ConnectionType` at the crate root. [#2171]
- `Response::into_body` that consumes response and returns body type. [#2201]
- `impl Default` for `Response`. [#2201]
- Add zstd support for `ContentEncoding`. [#2244]

### Changed

- The `MessageBody` trait now has an associated `Error` type. [#2183]
- All error trait bounds in server service builders have changed from `Into<Error>` to `Into<Response<AnyBody>>`. [#2253]
- All error trait bounds in message body and stream impls changed from `Into<Error>` to `Into<Box<dyn std::error::Error>>`. [#2253]
- Places in `Response` where `ResponseBody<B>` was received or returned now simply use `B`. [#2201]
- `header` mod is now public. [#2171]
- `uri` mod is now public. [#2171]
- Update `language-tags` to `0.3`.
- Reduce the level from `error` to `debug` for the log line that is emitted when a `500 Internal Server Error` is built using `HttpResponse::from_error`. [#2201]
- `ResponseBuilder::message_body` now returns a `Result`. [#2201]
- Remove `Unpin` bound on `ResponseBuilder::streaming`. [#2253]
- `HttpServer::{listen_rustls(), bind_rustls()}` now honor the ALPN protocols in the configuration parameter. [#2226]

### Removed

- Stop re-exporting `http` crate's `HeaderMap` types in addition to ours. [#2171]
- Down-casting for `MessageBody` types. [#2183]
- `error::Result` alias. [#2201]
- Error field from `Response` and `Response::error`. [#2205]
- `impl Future` for `Response`. [#2201]
- `Response::take_body` and old `Response::into_body` method that casted body type. [#2201]
- `InternalError` and all the error types it constructed. [#2215]
- Conversion (`impl Into`) of `Response<Body>` and `ResponseBuilder` to `Error`. [#2215]

[#2171]: https://github.com/actix/actix-web/pull/2171
[#2183]: https://github.com/actix/actix-web/pull/2183
[#2196]: https://github.com/actix/actix-web/pull/2196
[#2201]: https://github.com/actix/actix-web/pull/2201
[#2205]: https://github.com/actix/actix-web/pull/2205
[#2215]: https://github.com/actix/actix-web/pull/2215
[#2253]: https://github.com/actix/actix-web/pull/2253
[#2244]: https://github.com/actix/actix-web/pull/2244

## 3.0.0-beta.6

### Added

- `impl<T: MessageBody> MessageBody for Pin<Box<T>>`. [#2152]
- `Response::{ok, bad_request, not_found, internal_server_error}`. [#2159]
- Helper `body::to_bytes` for async collecting message body into Bytes. [#2158]

### Changed

- The type parameter of `Response` no longer has a default. [#2152]
- The `Message` variant of `body::Body` is now `Pin<Box<dyn MessageBody>>`. [#2152]
- `BodyStream` and `SizedStream` are no longer restricted to Unpin types. [#2152]
- Error enum types are marked `#[non_exhaustive]`. [#2161]

### Removed

- `cookies` feature flag. [#2065]
- Top-level `cookies` mod (re-export). [#2065]
- `HttpMessage` trait loses the `cookies` and `cookie` methods. [#2065]
- `impl ResponseError for CookieParseError`. [#2065]
- Deprecated methods on `ResponseBuilder`: `if_true`, `if_some`. [#2148]
- `ResponseBuilder::json`. [#2148]
- `ResponseBuilder::{set_header, header}`. [#2148]
- `impl From<serde_json::Value> for Body`. [#2148]
- `Response::build_from`. [#2159]
- Most of the status code builders on `Response`. [#2159]

[#2065]: https://github.com/actix/actix-web/pull/2065
[#2148]: https://github.com/actix/actix-web/pull/2148
[#2152]: https://github.com/actix/actix-web/pull/2152
[#2159]: https://github.com/actix/actix-web/pull/2159
[#2158]: https://github.com/actix/actix-web/pull/2158
[#2161]: https://github.com/actix/actix-web/pull/2161

## 3.0.0-beta.5

### Added

- `client::Connector::handshake_timeout` method for customizing TLS connection handshake timeout. [#2081]
- `client::ConnectorService` as `client::Connector::finish` method's return type [#2081]
- `client::ConnectionIo` trait alias [#2081]

### Changed

- `client::Connector` type now only have one generic type for `actix_service::Service`. [#2063]

### Removed

- Common typed HTTP headers were moved to actix-web. [2094]
- `ResponseError` impl for `actix_utils::timeout::TimeoutError`. [#2127]

[#2063]: https://github.com/actix/actix-web/pull/2063
[#2081]: https://github.com/actix/actix-web/pull/2081
[#2094]: https://github.com/actix/actix-web/pull/2094
[#2127]: https://github.com/actix/actix-web/pull/2127

## 3.0.0-beta.4

### Changed

- Feature `cookies` is now optional and disabled by default. [#1981]
- `ws::hash_key` now returns array. [#2035]
- `ResponseBuilder::json` now takes `impl Serialize`. [#2052]

### Removed

- Re-export of `futures_channel::oneshot::Canceled` is removed from `error` mod. [#1994]
- `ResponseError` impl for `futures_channel::oneshot::Canceled` is removed. [#1994]

[#1981]: https://github.com/actix/actix-web/pull/1981
[#1994]: https://github.com/actix/actix-web/pull/1994
[#2035]: https://github.com/actix/actix-web/pull/2035
[#2052]: https://github.com/actix/actix-web/pull/2052

## 3.0.0-beta.3

- No notable changes.

## 3.0.0-beta.2

### Added

- `TryIntoHeaderPair` trait that allows using typed and untyped headers in the same methods. [#1869]
- `ResponseBuilder::insert_header` method which allows using typed headers. [#1869]
- `ResponseBuilder::append_header` method which allows using typed headers. [#1869]
- `TestRequest::insert_header` method which allows using typed headers. [#1869]
- `ContentEncoding` implements all necessary header traits. [#1912]
- `HeaderMap::len_keys` has the behavior of the old `len` method. [#1964]
- `HeaderMap::drain` as an efficient draining iterator. [#1964]
- Implement `IntoIterator` for owned `HeaderMap`. [#1964]
- `trust-dns` optional feature to enable `trust-dns-resolver` as client dns resolver. [#1969]

### Changed

- `ResponseBuilder::content_type` now takes an `impl TryIntoHeaderValue` to support using typed `mime` types. [#1894]
- Renamed `TryIntoHeaderValue::{try_into => try_into_value}` to avoid ambiguity with std `TryInto` trait. [#1894]
- `Extensions::insert` returns Option of replaced item. [#1904]
- Remove `HttpResponseBuilder::json2()`. [#1903]
- Enable `HttpResponseBuilder::json()` to receive data by value and reference. [#1903]
- `client::error::ConnectError` Resolver variant contains `Box<dyn std::error::Error>` type. [#1905]
- `client::ConnectorConfig` default timeout changed to 5 seconds. [#1905]
- Simplify `BlockingError` type to a unit struct. It's now only triggered when blocking thread pool is dead. [#1957]
- `HeaderMap::len` now returns number of values instead of number of keys. [#1964]
- `HeaderMap::insert` now returns iterator of removed values. [#1964]
- `HeaderMap::remove` now returns iterator of removed values. [#1964]

### Removed

- `ResponseBuilder::set`; use `ResponseBuilder::insert_header`. [#1869]
- `ResponseBuilder::set_header`; use `ResponseBuilder::insert_header`. [#1869]
- `ResponseBuilder::header`; use `ResponseBuilder::append_header`. [#1869]
- `TestRequest::with_hdr`; use `TestRequest::default().insert_header()`. [#1869]
- `TestRequest::with_header`; use `TestRequest::default().insert_header()`. [#1869]
- `actors` optional feature. [#1969]
- `ResponseError` impl for `actix::MailboxError`. [#1969]

### Documentation

- Vastly improve docs and add examples for `HeaderMap`. [#1964]

[#1869]: https://github.com/actix/actix-web/pull/1869
[#1894]: https://github.com/actix/actix-web/pull/1894
[#1903]: https://github.com/actix/actix-web/pull/1903
[#1904]: https://github.com/actix/actix-web/pull/1904
[#1905]: https://github.com/actix/actix-web/pull/1905
[#1912]: https://github.com/actix/actix-web/pull/1912
[#1957]: https://github.com/actix/actix-web/pull/1957
[#1964]: https://github.com/actix/actix-web/pull/1964
[#1969]: https://github.com/actix/actix-web/pull/1969

## 3.0.0-beta.1

### Added

- Add `Http3` to `Protocol` enum for future compatibility and also mark `#[non_exhaustive]`.

### Changed

- Update `actix-*` dependencies to tokio `1.0` based versions. [#1813]
- Bumped `rand` to `0.8`.
- Update `bytes` to `1.0`. [#1813]
- Update `h2` to `0.3`. [#1813]
- The `ws::Message::Text` enum variant now contains a `bytestring::ByteString`. [#1864]

### Removed

- Deprecated `on_connect` methods have been removed. Prefer the new `on_connect_ext` technique. [#1857]
- Remove `ResponseError` impl for `actix::actors::resolver::ResolverError` due to deprecate of resolver actor. [#1813]
- Remove `ConnectError::SslHandshakeError` and re-export of `HandshakeError`. due to the removal of this type from `tokio-openssl` crate. openssl handshake error would return as `ConnectError::SslError`. [#1813]
- Remove `actix-threadpool` dependency. Use `actix_rt::task::spawn_blocking`. Due to this change `actix_threadpool::BlockingError` type is moved into `actix_http::error` module. [#1878]

[#1813]: https://github.com/actix/actix-web/pull/1813
[#1857]: https://github.com/actix/actix-web/pull/1857
[#1864]: https://github.com/actix/actix-web/pull/1864
[#1878]: https://github.com/actix/actix-web/pull/1878

</details>

## 2.2.2

### Changed

- Migrate to `brotli` crate. [ad7e3c06]

[ad7e3c06]: https://github.com/actix/actix-web/commit/ad7e3c06

## 2.2.1

### Fixed

- Potential HTTP request smuggling vulnerabilities. [RUSTSEC-2021-0081](https://github.com/rustsec/advisory-db/pull/977)

## 2.2.0

### Added

- HttpResponse builders for 1xx status codes. [#1768]
- `Accept::mime_precedence` and `Accept::mime_preference`. [#1793]
- `TryFrom<u16>` and `TryFrom<f32>` for `http::header::Quality`. [#1797]

### Fixed

- Started dropping `transfer-encoding: chunked` and `Content-Length` for 1XX and 204 responses. [#1767]

### Changed

- Upgrade `serde_urlencoded` to `0.7`. [#1773]

[#1773]: https://github.com/actix/actix-web/pull/1773
[#1767]: https://github.com/actix/actix-web/pull/1767
[#1768]: https://github.com/actix/actix-web/pull/1768
[#1793]: https://github.com/actix/actix-web/pull/1793
[#1797]: https://github.com/actix/actix-web/pull/1797

## 2.1.0

### Added

- Added more flexible `on_connect_ext` methods for on-connect handling. [#1754]

### Changed

- Upgrade `base64` to `0.13`. [#1744]
- Upgrade `pin-project` to `1.0`. [#1733]
- Deprecate `ResponseBuilder::{if_some, if_true}`. [#1760]

[#1760]: https://github.com/actix/actix-web/pull/1760
[#1754]: https://github.com/actix/actix-web/pull/1754
[#1733]: https://github.com/actix/actix-web/pull/1733
[#1744]: https://github.com/actix/actix-web/pull/1744

## 2.0.0

- No significant changes from `2.0.0-beta.4`.

## 2.0.0-beta.4

### Changed

- Update actix-codec and actix-utils dependencies.
- Update actix-connect and actix-tls dependencies.

## 2.0.0-beta.3

### Fixed

- Memory leak of `client::pool::ConnectorPoolSupport`. [#1626]

[#1626]: https://github.com/actix/actix-web/pull/1626

## 2.0.0-beta.2

### Fixed

- Potential UB in h1 decoder using uninitialized memory. [#1614]

### Changed

- Fix illegal chunked encoding. [#1615]

[#1614]: https://github.com/actix/actix-web/pull/1614
[#1615]: https://github.com/actix/actix-web/pull/1615

## 2.0.0-beta.1

### Changed

- Migrate cookie handling to `cookie` crate. [#1558]
- Update `sha-1` to 0.9. [#1586]
- Fix leak in client pool. [#1580]
- MSRV is now 1.41.1.

[#1558]: https://github.com/actix/actix-web/pull/1558
[#1586]: https://github.com/actix/actix-web/pull/1586
[#1580]: https://github.com/actix/actix-web/pull/1580

## 2.0.0-alpha.4

### Changed

- Bump minimum supported Rust version to 1.40
- content_length function is removed, and you can set Content-Length by calling no_chunking function [#1439]
- `BodySize::Sized64` variant has been removed. `BodySize::Sized` now receives a `u64` instead of a `usize`.
- Update `base64` dependency to 0.12

### Fixed

- Support parsing of `SameSite=None` [#1503]

[#1439]: https://github.com/actix/actix-web/pull/1439
[#1503]: https://github.com/actix/actix-web/pull/1503

## 2.0.0-alpha.3

### Fixed

- Correct spelling of ConnectError::Unresolved [#1487]
- Fix a mistake in the encoding of websocket continuation messages wherein Item::FirstText and Item::FirstBinary are each encoded as the other.

### Changed

- Implement `std::error::Error` for our custom errors [#1422]
- Remove `failure` support for `ResponseError` since that crate will be deprecated in the near future.

[#1422]: https://github.com/actix/actix-web/pull/1422
[#1487]: https://github.com/actix/actix-web/pull/1487

## 2.0.0-alpha.2

### Changed

- Update `actix-connect` and `actix-tls` dependency to 2.0.0-alpha.1. [#1395]
- Change default initial window size and connection window size for HTTP2 to 2MB and 1MB respectively to improve download speed for awc when downloading large objects. [#1394]
- client::Connector accepts initial_window_size and initial_connection_window_size HTTP2 configuration. [#1394]
- client::Connector allowing to set max_http_version to limit HTTP version to be used. [#1394]

[#1394]: https://github.com/actix/actix-web/pull/1394
[#1395]: https://github.com/actix/actix-web/pull/1395

## 2.0.0-alpha.1

### Changed

- Update the `time` dependency to 0.2.7.
- Moved actors messages support from actix crate, enabled with feature `actors`.
- Breaking change: trait MessageBody requires Unpin and accepting `Pin<&mut Self>` instead of `&mut self` in the poll_next().
- MessageBody is not implemented for &'static [u8] anymore.

### Fixed

- Allow `SameSite=None` cookies to be sent in a response.

## 1.0.1

### Fixed

- Poll upgrade service's readiness from HTTP service handlers
- Replace brotli with brotli2 #1224

## 1.0.0

### Added

- Add websockets continuation frame support

### Changed

- Replace `flate2-xxx` features with `compress`

## 1.0.0-alpha.5

### Fixed

- Check `Upgrade` service readiness before calling it
- Fix buffer remaining capacity calculation

### Changed

- Websockets: Ping and Pong should have binary data #1049

## 1.0.0-alpha.4

### Added

- Add impl ResponseBuilder for Error

### Changed

- Use rust based brotli compression library

## 1.0.0-alpha.3

### Changed

- Migrate to tokio 0.2
- Migrate to `std::future`

## 0.2.11

### Added

- Add support for serde_json::Value to be passed as argument to ResponseBuilder.body()
- Add an additional `filename*` param in the `Content-Disposition` header of `actix_files::NamedFile` to be more compatible. (#1151)
- Allow to use `std::convert::Infallible` as `actix_http::error::Error`

### Fixed

- To be compatible with non-English error responses, `ResponseError` rendered with `text/plain; charset=utf-8` header [#1118]

[#1878]: https://github.com/actix/actix-web/pull/1878

## 0.2.10

### Added

- Add support for sending HTTP requests with `Rc<RequestHead>` in addition to sending HTTP requests with `RequestHead`

### Fixed

- h2 will use error response #1080
- on_connect result isn't added to request extensions for http2 requests #1009

## 0.2.9

### Changed

- Dropped the `byteorder`-dependency in favor of `stdlib`-implementation
- Update percent-encoding to 2.1
- Update serde_urlencoded to 0.6.1

### Fixed

- Fixed a panic in the HTTP2 handshake in client HTTP requests (#1031)

## 0.2.8

### Added

- Add `rustls` support
- Add `Clone` impl for `HeaderMap`

### Fixed

- awc client panic #1016
- Invalid response with compression middleware enabled, but compression-related features disabled #997

## 0.2.7

### Added

- Add support for downcasting response errors #986

## 0.2.6

### Changed

- Replace `ClonableService` with local copy
- Upgrade `rand` dependency version to 0.7

## 0.2.5

### Added

- Add `on-connect` callback, `HttpServiceBuilder::on_connect()` #946

### Changed

- Use `encoding_rs` crate instead of unmaintained `encoding` crate
- Add `Copy` and `Clone` impls for `ws::Codec`

## 0.2.4

### Fixed

- Do not compress NoContent (204) responses #918

## 0.2.3

### Added

- Debug impl for ResponseBuilder
- From SizedStream and BodyStream for Body

### Changed

- SizedStream uses u64

## 0.2.2

### Fixed

- Parse incoming stream before closing stream on disconnect #868

## 0.2.1

### Fixed

- Handle socket read disconnect

## 0.2.0

### Changed

- Update actix-service to 0.4
- Expect and upgrade services accept `ServerConfig` config.

### Deleted

- `OneRequest` service

## 0.1.5

### Fixed

- Clean up response extensions in response pool #817

## 0.1.4

### Added

- Allow to render h1 request headers in `Camel-Case`

### Fixed

- Read until eof for http/1.0 responses #771

## 0.1.3

### Fixed

- Fix http client pool management
- Fix http client wait queue management #794

## 0.1.2

### Fixed

- Fix BorrowMutError panic in client connector #793

## 0.1.1

### Changed

- Cookie::max_age() accepts value in seconds
- Cookie::max_age_time() accepts value in time::Duration
- Allow to specify server address for client connector

## 0.1.0

### Added

- Expose peer addr via `Request::peer_addr()` and `RequestHead::peer_addr`

### Changed

- `actix_http::encoding` always available
- use trust-dns-resolver 0.11.0

## 0.1.0-alpha.5

### Added

- Allow to use custom service for upgrade requests
- Added `h1::SendResponse` future.

### Changed

- MessageBody::length() renamed to MessageBody::size() for consistency
- ws handshake verification functions take RequestHead instead of Request

## 0.1.0-alpha.4

### Added

- Allow to use custom `Expect` handler
- Add minimal `std::error::Error` impl for `Error`

### Changed

- Export IntoHeaderValue
- Render error and return as response body
- Use thread pool for response body compression

### Deleted

- Removed PayloadBuffer

## 0.1.0-alpha.3

### Added

- Warn when an unsealed private cookie isn't valid UTF-8

### Fixed

- Rust 1.31.0 compatibility
- Preallocate read buffer for h1 codec
- Detect socket disconnection during protocol selection

## 0.1.0-alpha.2

### Added

- Added ws::Message::Nop, no-op websockets message

### Changed

- Do not use thread pool for decompression if chunk size is smaller than 2048.

## 0.1.0-alpha.1

- Initial impl
