# Changes

## Unreleased

- Minimum supported Rust version (MSRV) is now 1.75.

## 0.6.6

- Update `tokio-uring` dependency to `0.4`.
- Minimum supported Rust version (MSRV) is now 1.72.

## 0.6.5

- Fix handling of special characters in filenames.

## 0.6.4

- Fix handling of newlines in filenames.
- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 0.6.3

- XHTML files now use `Content-Disposition: inline` instead of `attachment`. [#2903]
- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.
- Update `tokio-uring` dependency to `0.4`.

[#2903]: https://github.com/actix/actix-web/pull/2903

## 0.6.2

- Allow partial range responses for video content to start streaming sooner. [#2817]
- Minimum supported Rust version (MSRV) is now 1.57 due to transitive `time` dependency.

[#2817]: https://github.com/actix/actix-web/pull/2817

## 0.6.1

- Add `NamedFile::{modified, metadata, content_type, content_disposition, encoding}()` getters. [#2021]
- Update `tokio-uring` dependency to `0.3`.
- Audio files now use `Content-Disposition: inline` instead of `attachment`. [#2645]
- Minimum supported Rust version (MSRV) is now 1.56 due to transitive `hashbrown` dependency.

[#2021]: https://github.com/actix/actix-web/pull/2021
[#2645]: https://github.com/actix/actix-web/pull/2645

## 0.6.0

- No significant changes since `0.6.0-beta.16`.

## 0.6.0-beta.16

- No significant changes since `0.6.0-beta.15`.

## 0.6.0-beta.15

- No significant changes since `0.6.0-beta.14`.

## 0.6.0-beta.14

- The `prefer_utf8` option introduced in `0.4.0` is now true by default. [#2583]

[#2583]: https://github.com/actix/actix-web/pull/2583

## 0.6.0-beta.13

- The `Files` service now rejects requests with URL paths that include `%2F` (decoded: `/`). [#2398]
- The `Files` service now correctly decodes `%25` in the URL path to `%` for the file path. [#2398]
- Minimum supported Rust version (MSRV) is now 1.54.

[#2398]: https://github.com/actix/actix-web/pull/2398

## 0.6.0-beta.12

- No significant changes since `0.6.0-beta.11`.

## 0.6.0-beta.11

- No significant changes since `0.6.0-beta.10`.

## 0.6.0-beta.10

- No significant changes since `0.6.0-beta.9`.

## 0.6.0-beta.9

- Add crate feature `experimental-io-uring`, enabling async file I/O to be utilized. This feature is only available on Linux OSes with recent kernel versions. This feature is semver-exempt. [#2408]
- Add `NamedFile::open_async`. [#2408]
- Fix 304 Not Modified responses to omit the Content-Length header, as per the spec. [#2453]
- The `Responder` impl for `NamedFile` now has a boxed future associated type. [#2408]
- The `Service` impl for `NamedFileService` now has a boxed future associated type. [#2408]
- Add `impl Clone` for `FilesService`. [#2408]

[#2408]: https://github.com/actix/actix-web/pull/2408
[#2453]: https://github.com/actix/actix-web/pull/2453

## 0.6.0-beta.8

- Minimum supported Rust version (MSRV) is now 1.52.

## 0.6.0-beta.7

- Minimum supported Rust version (MSRV) is now 1.51.

## 0.6.0-beta.6

- Added `Files::path_filter()`. [#2274]
- `Files::show_files_listing()` can now be used with `Files::index_file()` to show files listing as a fallback when the index file is not found. [#2228]

[#2274]: https://github.com/actix/actix-web/pull/2274
[#2228]: https://github.com/actix/actix-web/pull/2228

## 0.6.0-beta.5

- `NamedFile` now implements `ServiceFactory` and `HttpServiceFactory` making it much more useful in routing. For example, it can be used directly as a default service. [#2135]
- For symbolic links, `Content-Disposition` header no longer shows the filename of the original file. [#2156]
- `Files::redirect_to_slash_directory()` now works as expected when used with `Files::show_files_listing()`. [#2225]
- `application/{javascript, json, wasm}` mime type now have `inline` disposition by default. [#2257]

[#2135]: https://github.com/actix/actix-web/pull/2135
[#2156]: https://github.com/actix/actix-web/pull/2156
[#2225]: https://github.com/actix/actix-web/pull/2225
[#2257]: https://github.com/actix/actix-web/pull/2257

## 0.6.0-beta.4

- Add support for `.guard` in `Files` to selectively filter `Files` services. [#2046]

[#2046]: https://github.com/actix/actix-web/pull/2046

## 0.6.0-beta.3

- No notable changes.

## 0.6.0-beta.2

- Fix If-Modified-Since and If-Unmodified-Since to not compare using sub-second timestamps. [#1887]
- Replace `v_htmlescape` with `askama_escape`. [#1953]

[#1887]: https://github.com/actix/actix-web/pull/1887
[#1953]: https://github.com/actix/actix-web/pull/1953

## 0.6.0-beta.1

- `HttpRange::parse` now has its own error type.
- Update `bytes` to `1.0`. [#1813]

[#1813]: https://github.com/actix/actix-web/pull/1813

## 0.5.0

- Optionally support hidden files/directories. [#1811]

[#1811]: https://github.com/actix/actix-web/pull/1811

## 0.4.1

- Clarify order of parameters in `Files::new` and improve docs.

## 0.4.0

- Add `Files::prefer_utf8` option that adds UTF-8 charset on certain response types. [#1714]

[#1714]: https://github.com/actix/actix-web/pull/1714

## 0.3.0

- No significant changes from 0.3.0-beta.1.

## 0.3.0-beta.1

- Update `v_htmlescape` to 0.10
- Update `actix-web` and `actix-http` dependencies to beta.1

## 0.3.0-alpha.1

- Update `actix-web` and `actix-http` dependencies to alpha
- Fix some typos in the docs
- Bump minimum supported Rust version to 1.40
- Support sending Content-Length when Content-Range is specified [#1384]

[#1384]: https://github.com/actix/actix-web/pull/1384

## 0.2.1

- Use the same format for file URLs regardless of platforms

## 0.2.0

- Fix BodyEncoding trait import #1220

## 0.2.0-alpha.1

- Migrate to `std::future`

## 0.1.7

- Add an additional `filename*` param in the `Content-Disposition` header of `actix_files::NamedFile` to be more compatible. (#1151)

## 0.1.6

- Add option to redirect to a slash-ended path `Files` #1132

## 0.1.5

- Bump up `mime_guess` crate version to 2.0.1
- Bump up `percent-encoding` crate version to 2.1
- Allow user defined request guards for `Files` #1113

## 0.1.4

- Allow to disable `Content-Disposition` header #686

## 0.1.3

- Do not set `Content-Length` header, let actix-http set it #930

## 0.1.2

- Content-Length is 0 for NamedFile HEAD request #914
- Fix ring dependency from actix-web default features for #741

## 0.1.1

- Static files are incorrectly served as both chunked and with length #812

## 0.1.0

- NamedFile last-modified check always fails due to nano-seconds in file modified date #820

## 0.1.0-beta.4

- Update actix-web to beta.4

## 0.1.0-beta.1

- Update actix-web to beta.1

## 0.1.0-alpha.6

- Update actix-web to alpha6

## 0.1.0-alpha.4

- Update actix-web to alpha4

## 0.1.0-alpha.2

- Add default handler support

## 0.1.0-alpha.1

- Initial impl
