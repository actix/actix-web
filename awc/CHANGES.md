# Changes

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
