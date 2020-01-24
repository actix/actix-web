# Changes

# [Unreleased]

### Fixed

* Allow `SameSite=None` cookies to be sent in a response.

## [1.0.1] - 2019-12-20

### Fixed

* Poll upgrade service's readiness from HTTP service handlers

* Replace brotli with brotli2 #1224

## [1.0.0] - 2019-12-13

### Added

* Add websockets continuation frame support

### Changed

* Replace `flate2-xxx` features with `compress`

## [1.0.0-alpha.5] - 2019-12-09

### Fixed

* Check `Upgrade` service readiness before calling it

* Fix buffer remaining capacity calcualtion

### Changed

* Websockets: Ping and Pong should have binary data #1049

## [1.0.0-alpha.4] - 2019-12-08

### Added

* Add impl ResponseBuilder for Error

### Changed

* Use rust based brotli compression library

## [1.0.0-alpha.3] - 2019-12-07

### Changed

* Migrate to tokio 0.2

* Migrate to `std::future`


## [0.2.11] - 2019-11-06

### Added

* Add support for serde_json::Value to be passed as argument to ResponseBuilder.body()

* Add an additional `filename*` param in the `Content-Disposition` header of `actix_files::NamedFile` to be more compatible. (#1151)

* Allow to use `std::convert::Infallible` as `actix_http::error::Error`

### Fixed

* To be compatible with non-English error responses, `ResponseError` rendered with `text/plain; charset=utf-8` header #1118


## [0.2.10] - 2019-09-11

### Added

* Add support for sending HTTP requests with `Rc<RequestHead>` in addition to sending HTTP requests with `RequestHead`

### Fixed

* h2 will use error response #1080

* on_connect result isn't added to request extensions for http2 requests #1009


## [0.2.9] - 2019-08-13

### Changed

* Dropped the `byteorder`-dependency in favor of `stdlib`-implementation

* Update percent-encoding to 2.1

* Update serde_urlencoded to 0.6.1

### Fixed

* Fixed a panic in the HTTP2 handshake in client HTTP requests (#1031)


## [0.2.8] - 2019-08-01

### Added

* Add `rustls` support

* Add `Clone` impl for `HeaderMap`

### Fixed

* awc client panic #1016

* Invalid response with compression middleware enabled, but compression-related features disabled #997


## [0.2.7] - 2019-07-18

### Added

* Add support for downcasting response errors #986


## [0.2.6] - 2019-07-17

### Changed

* Replace `ClonableService` with local copy

* Upgrade `rand` dependency version to 0.7


## [0.2.5] - 2019-06-28

### Added

* Add `on-connect` callback, `HttpServiceBuilder::on_connect()` #946

### Changed

* Use `encoding_rs` crate instead of unmaintained `encoding` crate

* Add `Copy` and `Clone` impls for `ws::Codec`


## [0.2.4] - 2019-06-16

### Fixed

* Do not compress NoContent (204) responses #918


## [0.2.3] - 2019-06-02

### Added

* Debug impl for ResponseBuilder

* From SizedStream and BodyStream for Body

### Changed

* SizedStream uses u64


## [0.2.2] - 2019-05-29

### Fixed

* Parse incoming stream before closing stream on disconnect #868


## [0.2.1] - 2019-05-25

### Fixed

* Handle socket read disconnect


## [0.2.0] - 2019-05-12

### Changed

* Update actix-service to 0.4

* Expect and upgrade services accept `ServerConfig` config.

### Deleted

* `OneRequest` service


## [0.1.5] - 2019-05-04

### Fixed

* Clean up response extensions in response pool #817


## [0.1.4] - 2019-04-24

### Added

* Allow to render h1 request headers in `Camel-Case`

### Fixed

* Read until eof for http/1.0 responses #771


## [0.1.3] - 2019-04-23

### Fixed

* Fix http client pool management

* Fix http client wait queue management #794


## [0.1.2] - 2019-04-23

### Fixed

* Fix BorrowMutError panic in client connector #793


## [0.1.1] - 2019-04-19

### Changed

* Cookie::max_age() accepts value in seconds

* Cookie::max_age_time() accepts value in time::Duration

* Allow to specify server address for client connector


## [0.1.0] - 2019-04-16

### Added

* Expose peer addr via `Request::peer_addr()` and `RequestHead::peer_addr`

### Changed

* `actix_http::encoding` always available

* use trust-dns-resolver 0.11.0


## [0.1.0-alpha.5] - 2019-04-12

### Added

* Allow to use custom service for upgrade requests

* Added `h1::SendResponse` future.

### Changed

* MessageBody::length() renamed to MessageBody::size() for consistency

* ws handshake verification functions take RequestHead instead of Request


## [0.1.0-alpha.4] - 2019-04-08

### Added

* Allow to use custom `Expect` handler

* Add minimal `std::error::Error` impl for `Error`

### Changed

* Export IntoHeaderValue

* Render error and return as response body

* Use thread pool for response body comression

### Deleted

* Removed PayloadBuffer


## [0.1.0-alpha.3] - 2019-04-02

### Added

* Warn when an unsealed private cookie isn't valid UTF-8

### Fixed

* Rust 1.31.0 compatibility

* Preallocate read buffer for h1 codec

* Detect socket disconnection during protocol selection


## [0.1.0-alpha.2] - 2019-03-29

### Added

* Added ws::Message::Nop, no-op websockets message

### Changed

* Do not use thread pool for decomression if chunk size is smaller than 2048.


## [0.1.0-alpha.1] - 2019-03-28

* Initial impl
