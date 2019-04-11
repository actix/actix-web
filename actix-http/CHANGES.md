# Changes

## [0.1.0-alpha.5] - 2019-04-xx

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
