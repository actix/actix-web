# Changes


## [0.1.0-alpha.3] - 2019-04-xx

### Added

* Export `MessageBody` type

* `ClientResponse::json()` - Loads and parse `application/json` encoded body


### Changed

* `ClientRequest::json()` accepts reference instead of object.

* `ClientResponse::body()` does not consume response object.


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
