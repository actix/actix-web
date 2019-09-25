# Changes

## [1.0.8] - 2019-09-25

### Added

* Add `Scope::register_data` and `Resource::register_data` methods, parallel to
  `App::register_data`.

* Add `middleware::Condition` that conditionally enables another middleware

* Allow to re-construct `ServiceRequest` from `HttpRequest` and `Payload`

* Add `HttpServer::listen_uds` for ability to listen on UDS FD rather than path,
  which is useful for example with systemd.

### Changed

* Make UrlEncodedError::Overflow more informativve

* Use actix-testing for testing utils


## [1.0.7] - 2019-08-29

### Fixed

* Request Extensions leak #1062


## [1.0.6] - 2019-08-28

### Added

* Re-implement Host predicate (#989)

* Form immplements Responder, returning a `application/x-www-form-urlencoded` response

* Add `into_inner` to `Data`

* Add `test::TestRequest::set_form()` convenience method to automatically serialize data and set
  the header in test requests.

### Changed

* `Query` payload made `pub`. Allows user to pattern-match the payload.

* Enable `rust-tls` feature for client #1045

* Update serde_urlencoded to 0.6.1

* Update url to 2.1


## [1.0.5] - 2019-07-18

### Added

* Unix domain sockets (HttpServer::bind_uds) #92

* Actix now logs errors resulting in "internal server error" responses always, with the `error`
  logging level

### Fixed

* Restored logging of errors through the `Logger` middleware


## [1.0.4] - 2019-07-17

### Added

* Add `Responder` impl for `(T, StatusCode) where T: Responder`

* Allow to access app's resource map via
  `ServiceRequest::resource_map()` and `HttpRequest::resource_map()` methods.

### Changed

* Upgrade `rand` dependency version to 0.7


## [1.0.3] - 2019-06-28

### Added

* Support asynchronous data factories #850

### Changed

*  Use `encoding_rs` crate instead of unmaintained `encoding` crate


## [1.0.2] - 2019-06-17

### Changed

* Move cors middleware to `actix-cors` crate.

* Move identity middleware to `actix-identity` crate.


## [1.0.1] - 2019-06-17

### Added

* Add support for PathConfig #903

* Add `middleware::identity::RequestIdentity` trait to `get_identity` from `HttpMessage`.

### Changed

* Move cors middleware to `actix-cors` crate.

* Move identity middleware to `actix-identity` crate.

* Disable default feature `secure-cookies`.

* Allow to test an app that uses async actors #897

* Re-apply patch from #637 #894

### Fixed

* HttpRequest::url_for is broken with nested scopes #915


## [1.0.0] - 2019-06-05

### Added

* Add `Scope::configure()` method.

* Add `ServiceRequest::set_payload()` method.

* Add `test::TestRequest::set_json()` convenience method to automatically
  serialize data and set header in test requests.

* Add macros for head, options, trace, connect and patch http methods

### Changed

* Drop an unnecessary `Option<_>` indirection around `ServerBuilder` from `HttpServer`. #863

### Fixed

* Fix Logger request time format, and use rfc3339. #867

* Clear http requests pool on app service drop #860


## [1.0.0-rc] - 2019-05-18

### Add

* Add `Query<T>::from_query()` to extract parameters from a query string. #846
* `QueryConfig`, similar to `JsonConfig` for customizing error handling of query extractors.

### Changed

* `JsonConfig` is now `Send + Sync`, this implies that `error_handler` must be `Send + Sync` too.

### Fixed

* Codegen with parameters in the path only resolves the first registered endpoint #841


## [1.0.0-beta.4] - 2019-05-12

### Add

* Allow to set/override app data on scope level

### Changed

* `App::configure` take an `FnOnce` instead of `Fn`
* Upgrade actix-net crates


## [1.0.0-beta.3] - 2019-05-04

### Added

* Add helper function for executing futures `test::block_fn()`

### Changed

* Extractor configuration could be registered with `App::data()`
  or with `Resource::data()` #775

* Route data is unified with app data, `Route::data()` moved to resource
  level to `Resource::data()`

* CORS handling without headers #702

* Allow to construct `Data` instances to avoid double `Arc` for `Send + Sync` types.

### Fixed

* Fix `NormalizePath` middleware impl #806

### Deleted

* `App::data_factory()` is deleted.


## [1.0.0-beta.2] - 2019-04-24

### Added

* Add raw services support via `web::service()`

* Add helper functions for reading response body `test::read_body()`

* Add support for `remainder match` (i.e "/path/{tail}*")

* Extend `Responder` trait, allow to override status code and headers.

* Store visit and login timestamp in the identity cookie #502

### Changed

* `.to_async()` handler can return `Responder` type #792

### Fixed

* Fix async web::Data factory handling


## [1.0.0-beta.1] - 2019-04-20

### Added

* Add helper functions for reading test response body,
 `test::read_response()` and test::read_response_json()`

* Add `.peer_addr()` #744

* Add `NormalizePath` middleware

### Changed

* Rename `RouterConfig` to `ServiceConfig`

* Rename `test::call_success` to `test::call_service`

* Removed `ServiceRequest::from_parts()` as it is unsafe to create from parts.

* `CookieIdentityPolicy::max_age()` accepts value in seconds

### Fixed

* Fixed `TestRequest::app_data()`


## [1.0.0-alpha.6] - 2019-04-14

### Changed

* Allow to use any service as default service.

* Remove generic type for request payload, always use default.

* Removed `Decompress` middleware. Bytes, String, Json, Form extractors
  automatically decompress payload.

* Make extractor config type explicit. Add `FromRequest::Config` associated type.


## [1.0.0-alpha.5] - 2019-04-12

### Added

* Added async io `TestBuffer` for testing.

### Deleted

* Removed native-tls support


## [1.0.0-alpha.4] - 2019-04-08

### Added

* `App::configure()` allow to offload app configuration to different methods

* Added `URLPath` option for logger

* Added `ServiceRequest::app_data()`, returns `Data<T>`

* Added `ServiceFromRequest::app_data()`, returns `Data<T>`

### Changed

* `FromRequest` trait refactoring

* Move multipart support to actix-multipart crate

### Fixed

* Fix body propagation in Response::from_error. #760


## [1.0.0-alpha.3] - 2019-04-02

### Changed

* Renamed `TestRequest::to_service()` to `TestRequest::to_srv_request()`

* Renamed `TestRequest::to_response()` to `TestRequest::to_srv_response()`

* Removed `Deref` impls

### Removed

* Removed unused `actix_web::web::md()`


## [1.0.0-alpha.2] - 2019-03-29

### Added

* rustls support

### Changed

* use forked cookie

* multipart::Field renamed to MultipartField

## [1.0.0-alpha.1] - 2019-03-28

### Changed

* Complete architecture re-design.

* Return 405 response if no matching route found within resource #538
