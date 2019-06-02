# Changes

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

### Changes

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
