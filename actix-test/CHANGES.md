# Changes

## Unreleased

## 0.1.5

- Add `TestServerConfig::listen_address()` method.

## 0.1.4

- Add `TestServerConfig::rustls_0_23()` method for Rustls v0.23 support behind new `rustls-0_23` crate feature.
- Add `TestServerConfig::disable_redirects()` method.
- Various types from `awc`, such as `ClientRequest` and `ClientResponse`, are now re-exported.
- Minimum supported Rust version (MSRV) is now 1.72.

## 0.1.3

- Add `TestServerConfig::rustls_0_22()` method for Rustls v0.22 support behind new `rustls-0_22` crate feature.

## 0.1.2

- Add `TestServerConfig::rustls_021()` method for Rustls v0.21 support behind new `rustls-0_21` crate feature.
- Add `TestServerConfig::workers()` method.
- Add `rustls-0_20` crate feature, which the existing `rustls` feature now aliases.
- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 0.1.1

- Add `TestServerConfig::port()` setter method.
- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

## 0.1.0

- Minimum supported Rust version (MSRV) is now 1.57 due to transitive `time` dependency.

## 0.1.0-beta.13

- No significant changes since `0.1.0-beta.12`.

## 0.1.0-beta.12

- Rename `TestServerConfig::{client_timeout => client_request_timeout}`. [#2611]

[#2611]: https://github.com/actix/actix-web/pull/2611

## 0.1.0-beta.11

- Minimum supported Rust version (MSRV) is now 1.54.

## 0.1.0-beta.10

- No significant changes since `0.1.0-beta.9`.

## 0.1.0-beta.9

- Re-export `actix_http::body::to_bytes`. [#2518]
- Update `actix_web::test` re-exports. [#2518]

[#2518]: https://github.com/actix/actix-web/pull/2518

## 0.1.0-beta.8

- No significant changes since `0.1.0-beta.7`.

## 0.1.0-beta.7

- Fix compatibility with experimental `io-uring` feature of `actix-rt`. [#2408]

[#2408]: https://github.com/actix/actix-web/pull/2408

## 0.1.0-beta.6

- No significant changes from `0.1.0-beta.5`.

## 0.1.0-beta.5

- Updated rustls to v0.20. [#2414]
- Minimum supported Rust version (MSRV) is now 1.52.

[#2414]: https://github.com/actix/actix-web/pull/2414

## 0.1.0-beta.4

- Minimum supported Rust version (MSRV) is now 1.51.

## 0.1.0-beta.3

- No significant changes from `0.1.0-beta.2`.

## 0.1.0-beta.2

- No significant changes from `0.1.0-beta.1`.

## 0.1.0-beta.1

- Move integration testing structs from `actix-web`. [#2112]

[#2112]: https://github.com/actix/actix-web/pull/2112
