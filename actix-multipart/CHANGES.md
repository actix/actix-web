# Changes

## [0.2.1] - 2020-01-xx

* Remove the unused `time` dependency

* Fix missing `std::error::Error` implement for `MultipartError`.

## [0.2.0] - 2019-12-20

* Release

## [0.2.0-alpha.4] - 2019-12-xx

* Multipart handling now handles Pending during read of boundary #1205

## [0.2.0-alpha.2] - 2019-12-03

* Migrate to `std::future`

## [0.1.4] - 2019-09-12

* Multipart handling now parses requests which do not end in CRLF #1038

## [0.1.3] - 2019-08-18

* Fix ring dependency from actix-web default features for #741.

## [0.1.2] - 2019-06-02

* Fix boundary parsing #876

## [0.1.1] - 2019-05-25

* Fix disconnect handling #834

## [0.1.0] - 2019-05-18

* Release

## [0.1.0-beta.4] - 2019-05-12

* Handle cancellation of uploads #736

* Upgrade to actix-web 1.0.0-beta.4

## [0.1.0-beta.1] - 2019-04-21

* Do not support nested multipart

* Split multipart support to separate crate

* Optimize multipart handling #634, #769
