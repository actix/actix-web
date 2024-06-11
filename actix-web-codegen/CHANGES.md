# Changes

## Unreleased

## 4.3.0

- Add `#[scope]` macro.
- Add `compat-routing-macros-force-pub` crate feature which, on-by-default, which when disabled causes handlers to inherit their attached function's visibility.
- Prevent inclusion of default `actix-router` features.
- Minimum supported Rust version (MSRV) is now 1.72.

## 4.2.2

- Fix regression when declaring `wrap` attribute using an expression.

## 4.2.1

- Update `syn` dependency to `2`.
- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 4.2.0

- Add support for custom methods with the `#[route]` macro. [#2969]

[#2969]: https://github.com/actix/actix-web/pull/2969

## 4.1.0

- Add `#[routes]` macro to support multiple paths for one handler. [#2718]
- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

[#2718]: https://github.com/actix/actix-web/pull/2718

## 4.0.1

- Fix support for guard paths in route handler macros. [#2771]
- Minimum supported Rust version (MSRV) is now 1.56 due to transitive `hashbrown` dependency.

[#2771]: https://github.com/actix/actix-web/pull/2771

## 4.0.0

- Version aligned with `actix-web` and will remain in sync going forward.
- No significant changes since `0.5.0`.

## 0.5.0

- No significant changes since `0.5.0-rc.2`.

## 0.5.0-rc.2

- No significant changes since `0.5.0-rc.1`.

## 0.5.0-rc.1

- Minimum supported Rust version (MSRV) is now 1.54.

## 0.5.0-beta.6

- No significant changes since `0.5.0-beta.5`.

## 0.5.0-beta.5

- Improve error recovery potential when macro input is invalid. [#2410]
- Add `#[actix_web::test]` macro for setting up tests with a runtime. [#2409]
- Minimum supported Rust version (MSRV) is now 1.52.

[#2410]: https://github.com/actix/actix-web/pull/2410
[#2409]: https://github.com/actix/actix-web/pull/2409

## 0.5.0-beta.4

- In routing macros, paths are now validated at compile time. [#2350]
- Minimum supported Rust version (MSRV) is now 1.51.

[#2350]: https://github.com/actix/actix-web/pull/2350

## 0.5.0-beta.3

- No notable changes.

## 0.5.0-beta.2

- Preserve doc comments when using route macros. [#2022]
- Add `name` attribute to `route` macro. [#1934]

[#2022]: https://github.com/actix/actix-web/pull/2022
[#1934]: https://github.com/actix/actix-web/pull/1934

## 0.5.0-beta.1

- Use new call signature for `System::new`.

## 0.4.0

- Added compile success and failure testing. [#1677]
- Add `route` macro for supporting multiple HTTP methods guards. [#1674]

[#1677]: https://github.com/actix/actix-web/pull/1677
[#1674]: https://github.com/actix/actix-web/pull/1674

## 0.3.0

- No significant changes from `0.3.0-beta.1`.

## 0.3.0-beta.1

- Add main entry-point macro that uses re-exported runtime. [#1559]

[#1559]: https://github.com/actix/actix-web/pull/1559

## 0.2.2

- Add resource middleware on actix-web-codegen [#1467]

[#1467]: https://github.com/actix/actix-web/pull/1467

## 0.2.1

- Add `#[allow(missing_docs)]` attribute to generated structs [#1368]
- Allow the handler function to be named as `config` [#1290]

[#1368]: https://github.com/actix/actix-web/issues/1368
[#1290]: https://github.com/actix/actix-web/issues/1290

## 0.2.0

- Generate code for actix-web 2.0

## 0.1.3

- Bump up `syn` & `quote` to 1.0
- Provide better error message

## 0.1.2

- Add macros for head, options, trace, connect and patch http methods

## 0.1.1

- Add syn "extra-traits" feature

## 0.1.0

- Release

## 0.1.0-beta.1

- Gen code for actix-web 1.0.0-beta.1

## 0.1.0-alpha.6

- Gen code for actix-web 1.0.0-alpha.6

## 0.1.0-alpha.1

- Initial impl
