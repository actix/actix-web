# Changes

## [0.1.6] - 2019-01-xx

### Changed

* Use `FnMut` instead of `Fn` for .apply() and .map() combinators and `FnService` type

## [0.1.5] - 2019-01-13

### Changed

* Make `Out::Error` convertable from `T::Error` for apply combinator


## [0.1.4] - 2019-01-11

### Changed

* Use `FnMut` instead of `Fn` for `FnService`


## [0.1.3] - 2018-12-12

### Changed

* Split service combinators to separate trait


## [0.1.2] - 2018-12-12

### Fixed

* Release future early for `.and_then()` and `.then()` combinators


## [0.1.1] - 2018-12-09

### Added

* Added Service impl for Box<S: Service>


## [0.1.0] - 2018-12-09

* Initial import
