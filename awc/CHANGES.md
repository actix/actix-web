# Changes

## Unreleased - 2020-xx-xx


## 2.0.2 - 2020-11-25
### Changed
* Upgrade `serde_urlencoded` to `0.7`. [#1773]

[#1773]: https://github.com/actix/actix-web/pull/1773


## 2.0.1 - 2020-10-30
### Changed
* Upgrade `base64` to `0.13`. [#1744]
* Deprecate `ClientRequest::{if_some, if_true}`. [#1760]

### Fixed
* Use `Accept-Encoding: identity` instead of `Accept-Encoding: br` when no compression feature
  is enabled [#1737]

[#1737]: https://github.com/actix/actix-web/pull/1737
[#1760]: https://github.com/actix/actix-web/pull/1760
[#1744]: https://github.com/actix/actix-web/pull/1744


## 2.0.0 - 2020-09-11
### Changed
* `Client::build` was renamed to `Client::builder`.


## 2.0.0-beta.4 - 2020-09-09
### Changed
* Update actix-codec & actix-tls dependencies.


## 2.0.0-beta.3 - 2020-08-17
### Changed
* Update `rustls` to 0.18


## 2.0.0-beta.2 - 2020-07-21
### Changed
* Update `actix-http` dependency to 2.0.0-beta.2


## [2.0.0-beta.1] - 2020-07-14
### Changed
* Update `actix-http` dependency to 2.0.0-beta.1

## [2.0.0-alpha.2] - 2020-05-21

### Changed

* Implement `std::error::Error` for our custom errors [#1422]
* Bump minimum supported Rust version to 1.40
* Update `base64` dependency to 0.12

[#1422]: https://github.com/actix/actix-web/pull/1422

## [2.0.0-alpha.1] - 2020-03-11

* Update `actix-http` dependency to 2.0.0-alpha.2
* Update `rustls` dependency to 0.17
* ClientBuilder accepts initial_window_size and initial_connection_window_size HTTP2 configuration
* ClientBuilder allowing to set max_http_version to limit HTTP version to be used

## [1.0.1] - 2019-12-15

* Fix compilation with default features off

## [1.0.0] - 2019-12-13

* Release

## [1.0.0-alpha.3]

* Migrate to `std::future`


## [0.2.8] - 2019-11-06

* Add support for setting query from Serialize type for client request.


## [0.2.7] - 2019-09-25

### Added

* Remaining getter methods for `ClientRequest`'s private `head` field #1101


## [0.2.6] - 2019-09-12

### Added

* Export frozen request related types.


## [0.2.5] - 2019-09-11

### Added

* Add `FrozenClientRequest` to support retries for sending HTTP requests

### Changed

* Ensure that the `Host` header is set when initiating a WebSocket client connection.


## [0.2.4] - 2019-08-13

### Changed

* Update percent-encoding to "2.1"

* Update serde_urlencoded to "0.6.1"


## [0.2.3] - 2019-08-01

### Added

* Add `rustls` support


## [0.2.2] - 2019-07-01

### Changed

* Always append a colon after username in basic auth

* Upgrade `rand` dependency version to 0.7


## [0.2.1] - 2019-06-05

### Added

* Add license files

## [0.2.0] - 2019-05-12

### Added

* Allow to send headers in `Camel-Case` form.

### Changed

* Upgrade actix-http dependency.


## [0.1.1] - 2019-04-19

### Added

* Allow to specify server address for http and ws requests.

### Changed

* `ClientRequest::if_true()` and `ClientRequest::if_some()` use instance instead of ref


## [0.1.0] - 2019-04-16

* No changes


## [0.1.0-alpha.6] - 2019-04-14

### Changed

* Do not set default headers for websocket request


## [0.1.0-alpha.5] - 2019-04-12

### Changed

* Do not set any default headers

### Added

* Add Debug impl for BoxedSocket


## [0.1.0-alpha.4] - 2019-04-08

### Changed

* Update actix-http dependency


## [0.1.0-alpha.3] - 2019-04-02

### Added

* Export `MessageBody` type

* `ClientResponse::json()` - Loads and parse `application/json` encoded body


### Changed

* `ClientRequest::json()` accepts reference instead of object.

* `ClientResponse::body()` does not consume response object.

* Renamed `ClientRequest::close_connection()` to `ClientRequest::force_close()`


## [0.1.0-alpha.2] - 2019-03-29

### Added

* Per request and session wide request timeout.

* Session wide headers.

* Session wide basic and bearer auth.

* Re-export `actix_http::client::Connector`.


### Changed

* Allow to override request's uri

* Export `ws` sub-module with websockets related types


## [0.1.0-alpha.1] - 2019-03-28

* Initial impl
