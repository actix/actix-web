# Changes

## Unreleased - 2022-xx-xx

## 4.2.0 - 2023-02-26

- Add support for custom methods with the `#[route]` macro. [#2969]

[#2969]: https://github.com/actix/actix-web/pull/2969

## 4.1.0 - 2022-09-11

- Add `#[routes]` macro to support multiple paths for one handler. [#2718]
- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

[#2718]: https://github.com/actix/actix-web/pull/2718

## 4.0.1 - 2022-06-11

- Fix support for guard paths in route handler macros. [#2771]
- Minimum supported Rust version (MSRV) is now 1.56 due to transitive `hashbrown` dependency.

[#2771]: https://github.com/actix/actix-web/pull/2771

## 4.0.0 - 2022-02-24

- Version aligned with `actix-web` and will remain in sync going forward.
- No significant changes since `0.5.0`.

## 0.5.0 - 2022-02-24

- No significant changes since `0.5.0-rc.2`.

## 0.5.0-rc.2 - 2022-02-01

- No significant changes since `0.5.0-rc.1`.

## 0.5.0-rc.1 - 2022-01-04

- Minimum supported Rust version (MSRV) is now 1.54.

## 0.5.0-beta.6 - 2021-12-11

- No significant changes since `0.5.0-beta.5`.

## 0.5.0-beta.5 - 2021-10-20

- Improve error recovery potential when macro input is invalid. [#2410]
- Add `#[actix_web::test]` macro for setting up tests with a runtime. [#2409]
- Minimum supported Rust version (MSRV) is now 1.52.

[#2410]: https://github.com/actix/actix-web/pull/2410
[#2409]: https://github.com/actix/actix-web/pull/2409

## 0.5.0-beta.4 - 2021-09-09

- In routing macros, paths are now validated at compile time. [#2350]
- Minimum supported Rust version (MSRV) is now 1.51.

[#2350]: https://github.com/actix/actix-web/pull/2350

## 0.5.0-beta.3 - 2021-06-17

- No notable changes.

## 0.5.0-beta.2 - 2021-03-09

- Preserve doc comments when using route macros. [#2022]
- Add `name` attribute to `route` macro. [#1934]

[#2022]: https://github.com/actix/actix-web/pull/2022
[#1934]: https://github.com/actix/actix-web/pull/1934

## 0.5.0-beta.1 - 2021-02-10

- Use new call signature for `System::new`.

## 0.4.0 - 2020-09-20

- Added compile success and failure testing. [#1677]
- Add `route` macro for supporting multiple HTTP methods guards. [#1674]

[#1677]: https://github.com/actix/actix-web/pull/1677
[#1674]: https://github.com/actix/actix-web/pull/1674

## 0.3.0 - 2020-09-11

- No significant changes from `0.3.0-beta.1`.

## 0.3.0-beta.1 - 2020-07-14

- Add main entry-point macro that uses re-exported runtime. [#1559]

[#1559]: https://github.com/actix/actix-web/pull/1559

## 0.2.2 - 2020-05-23

- Add resource middleware on actix-web-codegen [#1467]

[#1467]: https://github.com/actix/actix-web/pull/1467

## 0.2.1 - 2020-02-25

- Add `#[allow(missing_docs)]` attribute to generated structs [#1368]
- Allow the handler function to be named as `config` [#1290]

[#1368]: https://github.com/actix/actix-web/issues/1368
[#1290]: https://github.com/actix/actix-web/issues/1290

## 0.2.0 - 2019-12-13

- Generate code for actix-web 2.0

## 0.1.3 - 2019-10-14

- Bump up `syn` & `quote` to 1.0
- Provide better error message

## 0.1.2 - 2019-06-04

- Add macros for head, options, trace, connect and patch http methods

## 0.1.1 - 2019-06-01

- Add syn "extra-traits" feature

## 0.1.0 - 2019-05-18

- Release

## 0.1.0-beta.1 - 2019-04-20

- Gen code for actix-web 1.0.0-beta.1

## 0.1.0-alpha.6 - 2019-04-14

- Gen code for actix-web 1.0.0-alpha.6

## 0.1.0-alpha.1 - 2019-03-28

- Initial impl
