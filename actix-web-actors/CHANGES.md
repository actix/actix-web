# Changes

## Unreleased

## 4.3.1 <!-- v4.3.1+deprecated -->

- Reduce memory usage by `take`-ing (rather than `split`-ing) the encoded buffer when yielding bytes in the response stream.
- Mark crate as deprecated.
- Minimum supported Rust version (MSRV) is now 1.72.

## 4.3.0

- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 4.2.0

- Minimum supported Rust version (MSRV) is now 1.57 due to transitive `time` dependency.

## 4.1.0

- Add support for `actix` version `0.13`. [#2675]

[#2675]: https://github.com/actix/actix-web/pull/2675

## 4.0.0

- No significant changes since `4.0.0-beta.12`.

## 4.0.0-beta.12

- No significant changes since `4.0.0-beta.11`.

## 4.0.0-beta.11

- No significant changes since `4.0.0-beta.10`.

## 4.0.0-beta.10

- Minimum supported Rust version (MSRV) is now 1.54.

## 4.0.0-beta.9

- No significant changes since `4.0.0-beta.8`.

## 4.0.0-beta.8

- Add `ws:WsResponseBuilder` for building WebSocket session response. [#1920]
- Deprecate `ws::{start_with_addr, start_with_protocols}`. [#1920]
- Minimum supported Rust version (MSRV) is now 1.52.

[#1920]: https://github.com/actix/actix-web/pull/1920

## 4.0.0-beta.7

- Minimum supported Rust version (MSRV) is now 1.51.

## 4.0.0-beta.6

- Update `actix` to `0.12`. [#2277]

[#2277]: https://github.com/actix/actix-web/pull/2277

## 4.0.0-beta.5

- No notable changes.

## 4.0.0-beta.4

- No notable changes.

## 4.0.0-beta.3

- No notable changes.

## 4.0.0-beta.2

- No notable changes.

## 4.0.0-beta.1

- Update `pin-project` to `1.0`.
- Update `bytes` to `1.0`. [#1813]
- `WebsocketContext::text` now takes an `Into<bytestring::ByteString>`. [#1864]

[#1813]: https://github.com/actix/actix-web/pull/1813
[#1864]: https://github.com/actix/actix-web/pull/1864

## 3.0.0

- No significant changes from `3.0.0-beta.2`.

## 3.0.0-beta.2

- Update `actix-*` dependencies to latest versions.

## 3.0.0-beta.1

- Update `actix-web` & `actix-http` dependencies to beta.1
- Bump minimum supported Rust version to 1.40

## 3.0.0-alpha.1

- Update the actix-web dependency to 3.0.0-alpha.1
- Update the actix dependency to 0.10.0-alpha.2
- Update the actix-http dependency to 2.0.0-alpha.3

## 2.0.0

- Release

## 2.0.0-alpha.1

- Migrate to actix-web 2.0.0

## 1.0.4

- Allow comma-separated websocket subprotocols without spaces (#1172)

## 1.0.3

- Update actix-web and actix-http dependencies

## 1.0.2

- Add `ws::start_with_addr()`, returning the address of the created actor, along with the `HttpResponse`.

- Add support for specifying protocols on websocket handshake #835

## 1.0.1

- Allow to use custom ws codec with `WebsocketContext` #925

## 1.0.0

- Update actix-http and actix-web

## 0.1.0-alpha.3

- Update actix-http and actix-web

## 0.1.0-alpha.2

- Update actix-http and actix-web

## 0.1.0-alpha.1

- Initial impl
