# Changes

## Unreleased

- Minimum supported Rust version (MSRV) is now 1.75.

## 0.7.2

- Fix re-exported version of `actix-multipart-derive`.

## 0.7.1

- Expose `LimitExceeded` error type.

## 0.7.0

- Add `MultipartError::ContentTypeIncompatible` variant.
- Add `MultipartError::ContentDispositionNameMissing` variant.
- Add `Field::bytes()` method.
- Rename `MultipartError::{NoContentDisposition => ContentDispositionMissing}` variant.
- Rename `MultipartError::{NoContentType => ContentTypeMissing}` variant.
- Rename `MultipartError::{ParseContentType => ContentTypeParse}` variant.
- Rename `MultipartError::{Boundary => BoundaryMissing}` variant.
- Rename `MultipartError::{UnsupportedField => UnknownField}` variant.
- Remove top-level re-exports of `test` utilities.

## 0.6.2

- Add testing utilities under new module `test`.
- Minimum supported Rust version (MSRV) is now 1.72.

## 0.6.1

- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 0.6.0

- Added `MultipartForm` typed data extractor. [#2883]

[#2883]: https://github.com/actix/actix-web/pull/2883

## 0.5.0

- `Field::content_type()` now returns `Option<&mime::Mime>`. [#2885]
- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

[#2885]: https://github.com/actix/actix-web/pull/2885

## 0.4.0

- No significant changes since `0.4.0-beta.13`.

## 0.4.0-beta.13

- No significant changes since `0.4.0-beta.12`.

## 0.4.0-beta.12

- Minimum supported Rust version (MSRV) is now 1.54.

## 0.4.0-beta.11

- No significant changes since `0.4.0-beta.10`.

## 0.4.0-beta.10

- No significant changes since `0.4.0-beta.9`.

## 0.4.0-beta.9

- Polling `Field` after dropping `Multipart` now fails immediately instead of hanging forever. [#2463]

[#2463]: https://github.com/actix/actix-web/pull/2463

## 0.4.0-beta.8

- Ensure a correct Content-Disposition header is included in every part of a multipart message. [#2451]
- Added `MultipartError::NoContentDisposition` variant. [#2451]
- Since Content-Disposition is now ensured, `Field::content_disposition` is now infallible. [#2451]
- Added `Field::name` method for getting the field name. [#2451]
- `MultipartError` now marks variants with inner errors as the source. [#2451]
- `MultipartError` is now marked as non-exhaustive. [#2451]

[#2451]: https://github.com/actix/actix-web/pull/2451

## 0.4.0-beta.7

- Minimum supported Rust version (MSRV) is now 1.52.

## 0.4.0-beta.6

- Minimum supported Rust version (MSRV) is now 1.51.

## 0.4.0-beta.5

- No notable changes.

## 0.4.0-beta.4

- No notable changes.

## 0.4.0-beta.3

- No notable changes.

## 0.4.0-beta.2

- No notable changes.

## 0.4.0-beta.1

- Fix multipart consuming payload before header checks. [#1513]
- Update `bytes` to `1.0`. [#1813]

[#1813]: https://github.com/actix/actix-web/pull/1813
[#1513]: https://github.com/actix/actix-web/pull/1513

## 0.3.0

- No significant changes from `0.3.0-beta.2`.

## 0.3.0-beta.2

- Update `actix-*` dependencies to latest versions.

## 0.3.0-beta.1

- Update `actix-web` to 3.0.0-beta.1

## 0.3.0-alpha.1

- Update `actix-web` to 3.0.0-alpha.3
- Bump minimum supported Rust version to 1.40
- Minimize `futures` dependencies
- Remove the unused `time` dependency
- Fix missing `std::error::Error` implement for `MultipartError`.

## 0.2.0

- Release

## 0.2.0-alpha.4

- Multipart handling now handles Pending during read of boundary #1205

## 0.2.0-alpha.2

- Migrate to `std::future`

## 0.1.4

- Multipart handling now parses requests which do not end in CRLF #1038

## 0.1.3

- Fix ring dependency from actix-web default features for #741.

## 0.1.2

- Fix boundary parsing #876

## 0.1.1

- Fix disconnect handling #834

## 0.1.0

- Release

## 0.1.0-beta.4

- Handle cancellation of uploads #736

- Upgrade to actix-web 1.0.0-beta.4

## 0.1.0-beta.1

- Do not support nested multipart

- Split multipart support to separate crate

- Optimize multipart handling #634, #769
