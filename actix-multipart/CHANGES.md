# Changes

## Unreleased - 2021-xx-xx


## 0.4.0-beta.13 - 2022-01-31
- No significant changes since `0.4.0-beta.12`.


## 0.4.0-beta.12 - 2022-01-04
- Minimum supported Rust version (MSRV) is now 1.54.


## 0.4.0-beta.11 - 2021-12-27
- No significant changes since `0.4.0-beta.10`.


## 0.4.0-beta.10 - 2021-12-11
- No significant changes since `0.4.0-beta.9`.


## 0.4.0-beta.9 - 2021-12-01
- Polling `Field` after dropping `Multipart` now fails immediately instead of hanging forever. [#2463]

[#2463]: https://github.com/actix/actix-web/pull/2463


## 0.4.0-beta.8 - 2021-11-22
- Ensure a correct Content-Disposition header is included in every part of a multipart message. [#2451]
- Added `MultipartError::NoContentDisposition` variant. [#2451]
- Since Content-Disposition is now ensured, `Field::content_disposition` is now infallible. [#2451]
- Added `Field::name` method for getting the field name. [#2451]
- `MultipartError` now marks variants with inner errors as the source. [#2451]
- `MultipartError` is now marked as non-exhaustive. [#2451]

[#2451]: https://github.com/actix/actix-web/pull/2451


## 0.4.0-beta.7 - 2021-10-20
- Minimum supported Rust version (MSRV) is now 1.52.


## 0.4.0-beta.6 - 2021-09-09
- Minimum supported Rust version (MSRV) is now 1.51.


## 0.4.0-beta.5 - 2021-06-17
- No notable changes.


## 0.4.0-beta.4 - 2021-04-02
- No notable changes.


## 0.4.0-beta.3 - 2021-03-09
- No notable changes.


## 0.4.0-beta.2 - 2021-02-10
- No notable changes.


## 0.4.0-beta.1 - 2021-01-07
- Fix multipart consuming payload before header checks. [#1513]
- Update `bytes` to `1.0`. [#1813]

[#1813]: https://github.com/actix/actix-web/pull/1813
[#1513]: https://github.com/actix/actix-web/pull/1513


## 0.3.0 - 2020-09-11
- No significant changes from `0.3.0-beta.2`.


## 0.3.0-beta.2 - 2020-09-10
- Update `actix-*` dependencies to latest versions.


## 0.3.0-beta.1 - 2020-07-15
- Update `actix-web` to 3.0.0-beta.1


## 0.3.0-alpha.1 - 2020-05-25
- Update `actix-web` to 3.0.0-alpha.3
- Bump minimum supported Rust version to 1.40
- Minimize `futures` dependencies
- Remove the unused `time` dependency
- Fix missing `std::error::Error` implement for `MultipartError`.

## [0.2.0] - 2019-12-20

- Release

## [0.2.0-alpha.4] - 2019-12-xx

- Multipart handling now handles Pending during read of boundary #1205

## [0.2.0-alpha.2] - 2019-12-03

- Migrate to `std::future`

## [0.1.4] - 2019-09-12

- Multipart handling now parses requests which do not end in CRLF #1038

## [0.1.3] - 2019-08-18

- Fix ring dependency from actix-web default features for #741.

## [0.1.2] - 2019-06-02

- Fix boundary parsing #876

## [0.1.1] - 2019-05-25

- Fix disconnect handling #834

## [0.1.0] - 2019-05-18

- Release

## [0.1.0-beta.4] - 2019-05-12

- Handle cancellation of uploads #736

- Upgrade to actix-web 1.0.0-beta.4

## [0.1.0-beta.1] - 2019-04-21

- Do not support nested multipart

- Split multipart support to separate crate

- Optimize multipart handling #634, #769
