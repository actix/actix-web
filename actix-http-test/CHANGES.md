# Changes

## Unreleased

- Minimum supported Rust version (MSRV) is now 1.72.

## 3.2.0

- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 3.1.0

- Minimum supported Rust version (MSRV) is now 1.59.

## 3.0.0

- `TestServer::stop` is now async and will wait for the server and system to shutdown. [#2442]
- Added `TestServer::client_headers` method. [#2097]
- Update `actix-server` dependency to `2`.
- Update `actix-tls` dependency to `3`.
- Update `bytes` to `1.0`. [#1813]
- Minimum supported Rust version (MSRV) is now 1.57.

[#2442]: https://github.com/actix/actix-web/pull/2442
[#2097]: https://github.com/actix/actix-web/pull/2097
[#1813]: https://github.com/actix/actix-web/pull/1813

<details>
<summary>3.0.0 Pre-Releases</summary>

## 3.0.0-beta.13

- No significant changes since `3.0.0-beta.12`.

## 3.0.0-beta.12

- No significant changes since `3.0.0-beta.11`.

## 3.0.0-beta.11

- Minimum supported Rust version (MSRV) is now 1.54.

## 3.0.0-beta.10

- Update `actix-server` to `2.0.0-rc.2`. [#2550]

[#2550]: https://github.com/actix/actix-web/pull/2550

## 3.0.0-beta.9

- No significant changes since `3.0.0-beta.8`.

## 3.0.0-beta.8

- Update `actix-tls` to `3.0.0-rc.1`. [#2474]

[#2474]: https://github.com/actix/actix-web/pull/2474

## 3.0.0-beta.7

- Fix compatibility with experimental `io-uring` feature of `actix-rt`. [#2408]

[#2408]: https://github.com/actix/actix-web/pull/2408

## 3.0.0-beta.6

- `TestServer::stop` is now async and will wait for the server and system to shutdown. [#2442]
- Update `actix-server` to `2.0.0-beta.9`. [#2442]
- Minimum supported Rust version (MSRV) is now 1.52.

[#2442]: https://github.com/actix/actix-web/pull/2442

## 3.0.0-beta.5

- Minimum supported Rust version (MSRV) is now 1.51.

## 3.0.0-beta.4

- Added `TestServer::client_headers` method. [#2097]

[#2097]: https://github.com/actix/actix-web/pull/2097

## 3.0.0-beta.3

- No notable changes.

## 3.0.0-beta.2

- No notable changes.

## 3.0.0-beta.1

- Update `bytes` to `1.0`. [#1813]

[#1813]: https://github.com/actix/actix-web/pull/1813

</details>

## 2.1.0

- Add ability to set address for `TestServer`. [#1645]
- Upgrade `base64` to `0.13`.
- Upgrade `serde_urlencoded` to `0.7`. [#1773]

[#1773]: https://github.com/actix/actix-web/pull/1773
[#1645]: https://github.com/actix/actix-web/pull/1645

## 2.0.0

- Update actix-codec and actix-utils dependencies.

## 2.0.0-alpha.1

- Update the `time` dependency to 0.2.7
- Update `actix-connect` dependency to 2.0.0-alpha.2
- Make `test_server` `async` fn.
- Bump minimum supported Rust version to 1.40
- Replace deprecated `net2` crate with `socket2`
- Update `base64` dependency to 0.12
- Update `env_logger` dependency to 0.7

## 1.0.0

- Replaced `TestServer::start()` with `test_server()`

## 1.0.0-alpha.3

- Migrate to `std::future`

## 0.2.5

- Update serde_urlencoded to "0.6.1"
- Increase TestServerRuntime timeouts from 500ms to 3000ms
- Do not override current `System`

## 0.2.4

- Update actix-server to 0.6

## 0.2.3

- Add `delete`, `options`, `patch` methods to `TestServerRunner`

## 0.2.2

- Add .put() and .sput() methods

## 0.2.1

- Add license files

## 0.2.0

- Update awc and actix-http deps

## 0.1.1

- Always make new connection for http client

## 0.1.0

- No changes

## 0.1.0-alpha.3

- Request functions accept path #743

## 0.1.0-alpha.2

- Added TestServerRuntime::load_body() method
- Update actix-http and awc libraries

## 0.1.0-alpha.1

- Initial impl
