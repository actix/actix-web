# Changes

## [0.2.4] - 2018-11-21

### Added

* Allow to skip name resolution stage in Connector


## [0.2.3] - 2018-11-17

### Added

* Framed::is_write_buf_empty() checks if write buffer is flushed

## [0.2.2] - 2018-11-14

### Added

* Add low/high caps to Framed

### Changed

* Refactor Connector and Resolver services


### Fixed

* Fix wrong service to socket binding


## [0.2.0] - 2018-11-08

### Added

* Timeout service

* Added ServiceConfig and ServiceRuntime for server service configuration


### Changed

* Connector has been refactored

* timer and LowResTimer renamed to time and LowResTime

* Refactored `Server::configure()` method


## [0.1.1] - 2018-10-10

### Changed

- Set actix min version - 0.7.5

- Set trust-dns min version


## [0.1.0] - 2018-10-08

* Initial impl
