# Changes

## Unreleased

- Update `brotli` dependency to `7`.
- Prevent panics on connection pool drop when Tokio runtime is shutdown early.
- Minimum supported Rust version (MSRV) is now 1.75.
- Do not send `Host` header on HTTP/2 requests, as it is not required, and some web servers may reject it.

## 3.5.1

- Fix WebSocket `Host` request header value when using a non-default port.

## 3.5.0

- Add `rustls-0_23`, `rustls-0_23-webpki-roots`, and `rustls-0_23-native-roots` crate features.
- Add `awc::Connector::rustls_0_23()` constructor.
- Fix `rustls-0_22-native-roots` root store lookup.
- Update `brotli` dependency to `6`.
- Minimum supported Rust version (MSRV) is now 1.72.

## 3.4.0

- Add `rustls-0_22-webpki-roots` and `rustls-0_22-native-roots` crate feature.
- Add `awc::Connector::rustls_0_22()` method.

## 3.3.0

- Update `trust-dns-resolver` dependency to `0.23`.
- Updated `zstd` dependency to `0.13`.

## 3.2.0

- Add `awc::Connector::rustls_021()` method for Rustls v0.21 support behind new `rustls-0_21` crate feature.
- Add `rustls-0_20` crate feature, which the existing `rustls` feature now aliases.
- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 3.1.1

### Changed

- `client::Connect` is now public to allow tunneling connection with `client::Connector`.

## 3.1.0

### Changed

- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

## 3.0.1

### Changed

- Minimum supported Rust version (MSRV) is now 1.57 due to transitive `time` dependency.

### Fixed

- Fixed handling of redirection requests that begin with `//`. [#2840]

[#2840]: https://github.com/actix/actix-web/pull/2840

## 3.0.0

### Dependencies

- Updated `actix-*` to Tokio v1-based versions. [#1813]
- Updated `bytes` to `1.0`. [#1813]
- Updated `cookie` to `0.16`. [#2555]
- Updated `rand` to `0.8`.
- Updated `rustls` to `0.20`. [#2414]
- Updated `tokio` to `1`.

### Added

- `trust-dns` crate feature to enable `trust-dns-resolver` as client DNS resolver; disabled by default. [#1969]
- `cookies` crate feature; enabled by default. [#2619]
- `compress-brotli` crate feature; enabled by default. [#2250]
- `compress-gzip` crate feature; enabled by default. [#2250]
- `compress-zstd` crate feature; enabled by default. [#2250]
- `client::Connector::handshake_timeout()` for customizing TLS connection handshake timeout. [#2081]
- `client::ConnectorService` as `client::Connector::finish` method's return type [#2081]
- `client::ConnectionIo` trait alias [#2081]
- `Client::headers()` to get default mut reference of `HeaderMap` of client object. [#2114]
- `ClientResponse::timeout()` for set the timeout of collecting response body. [#1931]
- `ClientBuilder::local_address()` for binding to a local IP address for this client. [#2024]
- `ClientRequest::insert_header()` method which allows using typed and untyped headers. [#1869]
- `ClientRequest::append_header()` method which allows using typed and untyped headers. [#1869]
- `ClientBuilder::add_default_header()` (and deprecate `ClientBuilder::header()`). [#2510]

### Changed

- `client::Connector` type now only has one generic type for `actix_service::Service`. [#2063]
- `client::error::ConnectError` Resolver variant contains `Box<dyn std::error::Error>` type. [#1905]
- `client::ConnectorConfig` default timeout changed to 5 seconds. [#1905]
- `ConnectorService` type is renamed to `BoxConnectorService`. [#2081]
- Fix http/https encoding when enabling `compress` feature. [#2116]
- Rename `TestResponse::{header => append_header, set => insert_header}`. These methods now take a `TryIntoHeaderPair`. [#2094]
- `ClientBuilder::connector()` method now takes `Connector<T, U>` type. [#2008]
- Basic auth now accepts blank passwords as an empty string instead of an `Option`. [#2050]
- Relax default timeout for `Connector` to 5 seconds (up from 1 second). [#1905]
- `*::send_json()` and `*::send_form()` methods now receive `impl Serialize`. [#2553]
- `FrozenClientRequest::extra_header()` now uses receives an `impl TryIntoHeaderPair`. [#2553]
- Rename `Connector::{ssl => openssl}()`. [#2503]
- `ClientRequest::send_body` now takes an `impl MessageBody`. [#2546]
- Rename `MessageBody => ResponseBody` to avoid conflicts with `MessageBody` trait. [#2546]
- Minimum supported Rust version (MSRV) is now 1.54.

### Fixed

- Send headers along with redirected requests. [#2310]
- Improve `Client` instantiation efficiency when using `openssl` by only building connectors once. [#2503]
- Remove unnecessary `Unpin` bounds on `*::send_stream`. [#2553]
- `impl Future` for `ResponseBody` no longer requires the body type be `Unpin`. [#2546]
- `impl Future` for `JsonBody` no longer requires the body type be `Unpin`. [#2546]
- `impl Stream` for `ClientResponse` no longer requires the body type be `Unpin`. [#2546]

### Removed

- `compress` crate feature. [#2250]
- `ClientRequest::set`; use `ClientRequest::insert_header`. [#1869]
- `ClientRequest::set_header`; use `ClientRequest::insert_header`. [#1869]
- `ClientRequest::set_header_if_none`; use `ClientRequest::insert_header_if_none`. [#1869]
- `ClientRequest::header`; use `ClientRequest::append_header`. [#1869]
- Deprecated methods on `ClientRequest`: `if_true`, `if_some`. [#2148]
- `ClientBuilder::default` function [#2008]

### Security

- `cookie` upgrade addresses [`RUSTSEC-2020-0071`].

[`rustsec-2020-0071`]: https://rustsec.org/advisories/RUSTSEC-2020-0071.html
[#1813]: https://github.com/actix/actix-web/pull/1813
[#1869]: https://github.com/actix/actix-web/pull/1869
[#1905]: https://github.com/actix/actix-web/pull/1905
[#1905]: https://github.com/actix/actix-web/pull/1905
[#1931]: https://github.com/actix/actix-web/pull/1931
[#1969]: https://github.com/actix/actix-web/pull/1969
[#1969]: https://github.com/actix/actix-web/pull/1969
[#1981]: https://github.com/actix/actix-web/pull/1981
[#2008]: https://github.com/actix/actix-web/pull/2008
[#2024]: https://github.com/actix/actix-web/pull/2024
[#2050]: https://github.com/actix/actix-web/pull/2050
[#2063]: https://github.com/actix/actix-web/pull/2063
[#2081]: https://github.com/actix/actix-web/pull/2081
[#2081]: https://github.com/actix/actix-web/pull/2081
[#2094]: https://github.com/actix/actix-web/pull/2094
[#2114]: https://github.com/actix/actix-web/pull/2114
[#2116]: https://github.com/actix/actix-web/pull/2116
[#2148]: https://github.com/actix/actix-web/pull/2148
[#2250]: https://github.com/actix/actix-web/pull/2250
[#2310]: https://github.com/actix/actix-web/pull/2310
[#2414]: https://github.com/actix/actix-web/pull/2414
[#2425]: https://github.com/actix/actix-web/pull/2425
[#2474]: https://github.com/actix/actix-web/pull/2474
[#2503]: https://github.com/actix/actix-web/pull/2503
[#2510]: https://github.com/actix/actix-web/pull/2510
[#2546]: https://github.com/actix/actix-web/pull/2546
[#2553]: https://github.com/actix/actix-web/pull/2553
[#2555]: https://github.com/actix/actix-web/pull/2555

<details>
<summary>3.0.0 Pre-Releases</summary>

## 3.0.0-beta.21

- No significant changes since `3.0.0-beta.20`.

## 3.0.0-beta.20

- No significant changes since `3.0.0-beta.19`.

## 3.0.0-beta.19

- No significant changes since `3.0.0-beta.18`.

## 3.0.0-beta.18

- Minimum supported Rust version (MSRV) is now 1.54.

## 3.0.0-beta.17

### Changed

- Update `cookie` dependency (re-exported) to `0.16`. [#2555]

### Security

- `cookie` upgrade addresses [`RUSTSEC-2020-0071`].

[#2555]: https://github.com/actix/actix-web/pull/2555
[`rustsec-2020-0071`]: https://rustsec.org/advisories/RUSTSEC-2020-0071.html

## 3.0.0-beta.16

- `*::send_json` and `*::send_form` methods now receive `impl Serialize`. [#2553]
- `FrozenClientRequest::extra_header` now uses receives an `impl TryIntoHeaderPair`. [#2553]
- Remove unnecessary `Unpin` bounds on `*::send_stream`. [#2553]

[#2553]: https://github.com/actix/actix-web/pull/2553

## 3.0.0-beta.15

- Rename `Connector::{ssl => openssl}`. [#2503]
- Improve `Client` instantiation efficiency when using `openssl` by only building connectors once. [#2503]
- `ClientRequest::send_body` now takes an `impl MessageBody`. [#2546]
- Rename `MessageBody => ResponseBody` to avoid conflicts with `MessageBody` trait. [#2546]
- `impl Future` for `ResponseBody` no longer requires the body type be `Unpin`. [#2546]
- `impl Future` for `JsonBody` no longer requires the body type be `Unpin`. [#2546]
- `impl Stream` for `ClientResponse` no longer requires the body type be `Unpin`. [#2546]

[#2503]: https://github.com/actix/actix-web/pull/2503
[#2546]: https://github.com/actix/actix-web/pull/2546

## 3.0.0-beta.14

- Add `ClientBuilder::add_default_header` and deprecate `ClientBuilder::header`. [#2510]

[#2510]: https://github.com/actix/actix-web/pull/2510

## 3.0.0-beta.13

- No significant changes since `3.0.0-beta.12`.

## 3.0.0-beta.12

- Update `actix-tls` to `3.0.0-rc.1`. [#2474]

[#2474]: https://github.com/actix/actix-web/pull/2474

## 3.0.0-beta.11

- No significant changes from `3.0.0-beta.10`.

## 3.0.0-beta.10

- No significant changes from `3.0.0-beta.9`.

## 3.0.0-beta.9

- Updated rustls to v0.20. [#2414]

[#2414]: https://github.com/actix/actix-web/pull/2414

## 3.0.0-beta.8

### Changed

- Send headers within the redirect requests. [#2310]

[#2310]: https://github.com/actix/actix-web/pull/2310

## 3.0.0-beta.7

### Changed

- Change compression algorithm features flags. [#2250]

[#2250]: https://github.com/actix/actix-web/pull/2250

## 3.0.0-beta.6

- No significant changes since 3.0.0-beta.5.

## 3.0.0-beta.5

### Removed

- Deprecated methods on `ClientRequest`: `if_true`, `if_some`. [#2148]

[#2148]: https://github.com/actix/actix-web/pull/2148

## 3.0.0-beta.4

### Added

- Add `Client::headers` to get default mut reference of `HeaderMap` of client object. [#2114]

### Changed

- `ConnectorService` type is renamed to `BoxConnectorService`. [#2081]
- Fix http/https encoding when enabling `compress` feature. [#2116]
- Rename `TestResponse::header` to `append_header`, `set` to `insert_header`. `TestResponse` header methods now take `TryIntoHeaderPair` tuples. [#2094]

[#2081]: https://github.com/actix/actix-web/pull/2081
[#2094]: https://github.com/actix/actix-web/pull/2094
[#2114]: https://github.com/actix/actix-web/pull/2114
[#2116]: https://github.com/actix/actix-web/pull/2116

## 3.0.0-beta.3

### Added

- `ClientResponse::timeout` for set the timeout of collecting response body. [#1931]
- `ClientBuilder::local_address` for bind to a local ip address for this client. [#2024]

### Changed

- Feature `cookies` is now optional and enabled by default. [#1981]
- `ClientBuilder::connector` method would take `actix_http::client::Connector<T, U>` type. [#2008]
- Basic auth password now takes blank passwords as an empty string instead of Option. [#2050]

### Removed

- `ClientBuilder::default` function [#2008]

[#1931]: https://github.com/actix/actix-web/pull/1931
[#1981]: https://github.com/actix/actix-web/pull/1981
[#2008]: https://github.com/actix/actix-web/pull/2008
[#2024]: https://github.com/actix/actix-web/pull/2024
[#2050]: https://github.com/actix/actix-web/pull/2050

## 3.0.0-beta.2

### Added

- `ClientRequest::insert_header` method which allows using typed headers. [#1869]
- `ClientRequest::append_header` method which allows using typed headers. [#1869]
- `trust-dns` optional feature to enable `trust-dns-resolver` as client dns resolver. [#1969]

### Changed

- Relax default timeout for `Connector` to 5 seconds(original 1 second). [#1905]

### Removed

- `ClientRequest::set`; use `ClientRequest::insert_header`. [#1869]
- `ClientRequest::set_header`; use `ClientRequest::insert_header`. [#1869]
- `ClientRequest::set_header_if_none`; use `ClientRequest::insert_header_if_none`. [#1869]
- `ClientRequest::header`; use `ClientRequest::append_header`. [#1869]

[#1869]: https://github.com/actix/actix-web/pull/1869
[#1905]: https://github.com/actix/actix-web/pull/1905
[#1969]: https://github.com/actix/actix-web/pull/1969

## 3.0.0-beta.1

### Changed

- Update `rand` to `0.8`
- Update `bytes` to `1.0`. [#1813]
- Update `rust-tls` to `0.19`. [#1813]

[#1813]: https://github.com/actix/actix-web/pull/1813

</details>

## 2.0.3

### Fixed

- Ensure `actix-http` dependency uses same `serde_urlencoded`.

## 2.0.2

### Changed

- Upgrade `serde_urlencoded` to `0.7`. [#1773]

[#1773]: https://github.com/actix/actix-web/pull/1773

## 2.0.1

### Changed

- Upgrade `base64` to `0.13`. [#1744]
- Deprecate `ClientRequest::{if_some, if_true}`. [#1760]

### Fixed

- Use `Accept-Encoding: identity` instead of `Accept-Encoding: br` when no compression feature is enabled [#1737]

[#1737]: https://github.com/actix/actix-web/pull/1737
[#1760]: https://github.com/actix/actix-web/pull/1760
[#1744]: https://github.com/actix/actix-web/pull/1744

## 2.0.0

### Changed

- `Client::build` was renamed to `Client::builder`.

## 2.0.0-beta.4

### Changed

- Update actix-codec & actix-tls dependencies.

## 2.0.0-beta.3

### Changed

- Update `rustls` to 0.18

## 2.0.0-beta.2

### Changed

- Update `actix-http` dependency to 2.0.0-beta.2

## 2.0.0-beta.1

### Changed

- Update `actix-http` dependency to 2.0.0-beta.1

## 2.0.0-alpha.2

### Changed

- Implement `std::error::Error` for our custom errors [#1422]
- Bump minimum supported Rust version to 1.40
- Update `base64` dependency to 0.12

[#1422]: https://github.com/actix/actix-web/pull/1422

## 2.0.0-alpha.1

- Update `actix-http` dependency to 2.0.0-alpha.2
- Update `rustls` dependency to 0.17
- ClientBuilder accepts initial_window_size and initial_connection_window_size HTTP2 configuration
- ClientBuilder allowing to set max_http_version to limit HTTP version to be used

## 1.0.1

- Fix compilation with default features off

## 1.0.0

- Release

## 1.0.0-alpha.3

- Migrate to `std::future`

## 0.2.8

- Add support for setting query from Serialize type for client request.

## 0.2.7

### Added

- Remaining getter methods for `ClientRequest`'s private `head` field #1101

## 0.2.6

### Added

- Export frozen request related types.

## 0.2.5

### Added

- Add `FrozenClientRequest` to support retries for sending HTTP requests

### Changed

- Ensure that the `Host` header is set when initiating a WebSocket client connection.

## 0.2.4

### Changed

- Update percent-encoding to "2.1"

- Update serde_urlencoded to "0.6.1"

## 0.2.3

### Added

- Add `rustls` support

## 0.2.2

### Changed

- Always append a colon after username in basic auth

- Upgrade `rand` dependency version to 0.7

## 0.2.1

### Added

- Add license files

## 0.2.0

### Added

- Allow to send headers in `Camel-Case` form.

### Changed

- Upgrade actix-http dependency.

## 0.1.1

### Added

- Allow to specify server address for http and ws requests.

### Changed

- `ClientRequest::if_true()` and `ClientRequest::if_some()` use instance instead of ref

## 0.1.0

- No changes

## 0.1.0-alpha.6

### Changed

- Do not set default headers for websocket request

## 0.1.0-alpha.5

### Changed

- Do not set any default headers

### Added

- Add Debug impl for BoxedSocket

## 0.1.0-alpha.4

### Changed

- Update actix-http dependency

## 0.1.0-alpha.3

### Added

- Export `MessageBody` type

- `ClientResponse::json()` - Loads and parse `application/json` encoded body

### Changed

- `ClientRequest::json()` accepts reference instead of object.

- `ClientResponse::body()` does not consume response object.

- Renamed `ClientRequest::close_connection()` to `ClientRequest::force_close()`

## 0.1.0-alpha.2

### Added

- Per request and session wide request timeout.

- Session wide headers.

- Session wide basic and bearer auth.

- Re-export `actix_http::client::Connector`.

### Changed

- Allow to override request's uri

- Export `ws` sub-module with websockets related types

## 0.1.0-alpha.1

- Initial impl
