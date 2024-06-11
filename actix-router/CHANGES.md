# Changes

## Unreleased

## 0.5.3

- Add `unicode` crate feature (on-by-default) to switch between `regex` and `regex-lite` as a trade-off between full unicode support and binary size.
- Minimum supported Rust version (MSRV) is now 1.72.

## 0.5.2

- Minimum supported Rust version (MSRV) is now 1.68 due to transitive `time` dependency.

## 0.5.1

- Correct typo in error string for `i32` deserialization. [#2876]
- Minimum supported Rust version (MSRV) is now 1.59 due to transitive `time` dependency.

[#2876]: https://github.com/actix/actix-web/pull/2876

## 0.5.0

### Added

- Add `Path::as_str`. [#2590]
- Add `ResourceDef::set_name`. [#373][net#373]
- Add `RouterBuilder::push`. [#2612]
- Implement `IntoPatterns` for `bytestring::ByteString`. [#372][net#372]
- Introduce `ResourceDef::join`. [#380][net#380]
- Introduce `ResourceDef::pattern_iter` to get an iterator over all patterns in a multi-pattern resource. [#373][net#373]
- `Resource` is now implemented for `&mut Path<_>` and `RefMut<Path<_>>`. [#2568]
- Support `build_resource_path` on multi-pattern resources. [#2356]
- Support multi-pattern prefixes and joins. [#2356]

### Changed

- Change signature of `ResourceDef::capture_match_info_fn` to remove `user_data` parameter. [#2612]
- Deprecate `Path::path`. [#2590]
- Disallow prefix routes with tail segments. [#379][net#379]
- Enforce path separators on dynamic prefixes. [#378][net#378]
- Minimum supported Rust version (MSRV) is now 1.54.
- Prefix segments now always end with with a segment delimiter or end-of-input. [#2355]
- Prefix segments with trailing slashes define a trailing empty segment. [#2355]
- `Quoter::requote` now returns `Option<Vec<u8>>`. [#2613]
- Re-work `IntoPatterns` trait, adding a `Patterns` enum. [#372][net#372]
- Rename `Path::{len => segment_count}` to be more descriptive of its purpose. [#370][net#370]
- Rename `ResourceDef::{is_prefix_match => find_match}`. [#373][net#373]
- Rename `ResourceDef::{match_path => capture_match_info}`. [#373][net#373]
- Rename `ResourceDef::{match_path_checked => capture_match_info_fn}`. [#373][net#373]
- Rename `ResourceDef::{resource_path => resource_path_from_iter}`. [#371][net#371]
- Rename `ResourceDef::{resource_path_named => resource_path_from_map}`. [#371][net#371]
- Rename `Router::{*_checked => *_fn}`. [#373][net#373]
- Replace `Option<U>` with `U` in `Router` API. [#2612]
- `Resource` trait now uses an associated type, `Path`, instead of a generic parameter. [#2568]
- `ResourceDef::pattern` now returns the first pattern in multi-pattern resources. [#2356]
- `ResourceDef::resource_path_from_iter` now takes an `IntoIterator`. [#373][net#373]
- Return type of `ResourceDef::name` is now `Option<&str>`. [#373][net#373]
- Return type of `ResourceDef::pattern` is now `Option<&str>`. [#373][net#373]

### Fixed

- Fix `ResourceDef`'s `PartialEq` implementation. [#373][net#373]
- Fix segment interpolation leaving `Path` in unintended state after matching. [#368][net#368]
- Improve malformed path error message. [#384][net#384]
- `PathDeserializer` now decodes all percent encoded characters in dynamic segments. [#2566]
- Relax bounds on `Router::recognize*` and `ResourceDef::capture_match_info`. [#2612]
- Static patterns in multi-patterns are no longer interpreted as regex. [#366][net#366]

### Removed

- `ResourceDef::name_mut`. [#373][net#373]
- Unused `ResourceInfo`. [#2612]

[#2355]: https://github.com/actix/actix-web/pull/2355
[#2356]: https://github.com/actix/actix-web/pull/2356
[#2566]: https://github.com/actix/actix-net/pull/2566
[#2568]: https://github.com/actix/actix-web/pull/2568
[#2590]: https://github.com/actix/actix-web/pull/2590
[#2612]: https://github.com/actix/actix-web/pull/2612
[#2613]: https://github.com/actix/actix-web/pull/2613
[net#366]: https://github.com/actix/actix-net/pull/366
[net#368]: https://github.com/actix/actix-net/pull/368
[net#368]: https://github.com/actix/actix-net/pull/368
[net#370]: https://github.com/actix/actix-net/pull/370
[net#371]: https://github.com/actix/actix-net/pull/371
[net#372]: https://github.com/actix/actix-net/pull/372
[net#373]: https://github.com/actix/actix-net/pull/373
[net#378]: https://github.com/actix/actix-net/pull/378
[net#379]: https://github.com/actix/actix-net/pull/379
[net#380]: https://github.com/actix/actix-net/pull/380
[net#384]: https://github.com/actix/actix-net/pull/384

<details>
<summary>0.5.0 Pre-Releases</summary>

## 0.5.0-rc.3

- Remove unused `ResourceInfo`. [#2612]
- Add `RouterBuilder::push`. [#2612]
- Change signature of `ResourceDef::capture_match_info_fn` to remove `user_data` parameter. [#2612]
- Replace `Option<U>` with `U` in `Router` API. [#2612]
- Relax bounds on `Router::recognize*` and `ResourceDef::capture_match_info`. [#2612]
- `Quoter::requote` now returns `Option<Vec<u8>>`. [#2613]

[#2612]: https://github.com/actix/actix-web/pull/2612
[#2613]: https://github.com/actix/actix-web/pull/2613

## 0.5.0-rc.2

- Add `Path::as_str`. [#2590]
- Deprecate `Path::path`. [#2590]

[#2590]: https://github.com/actix/actix-web/pull/2590

## 0.5.0-rc.1

- `Resource` trait now have an associated type, `Path`, instead of the generic parameter. [#2568]
- `Resource` is now implemented for `&mut Path<_>` and `RefMut<Path<_>>`. [#2568]

[#2568]: https://github.com/actix/actix-web/pull/2568

## 0.5.0-beta.4

- `PathDeserializer` now decodes all percent encoded characters in dynamic segments. [#2566]
- Minimum supported Rust version (MSRV) is now 1.54.

[#2566]: https://github.com/actix/actix-net/pull/2566

## 0.5.0-beta.3

- Minimum supported Rust version (MSRV) is now 1.52.

## 0.5.0-beta.2

- Introduce `ResourceDef::join`. [#380][net#380]
- Disallow prefix routes with tail segments. [#379][net#379]
- Enforce path separators on dynamic prefixes. [#378][net#378]
- Improve malformed path error message. [#384][net#384]
- Prefix segments now always end with with a segment delimiter or end-of-input. [#2355]
- Prefix segments with trailing slashes define a trailing empty segment. [#2355]
- Support multi-pattern prefixes and joins. [#2356]
- `ResourceDef::pattern` now returns the first pattern in multi-pattern resources. [#2356]
- Support `build_resource_path` on multi-pattern resources. [#2356]
- Minimum supported Rust version (MSRV) is now 1.51.

[net#378]: https://github.com/actix/actix-net/pull/378
[net#379]: https://github.com/actix/actix-net/pull/379
[net#380]: https://github.com/actix/actix-net/pull/380
[net#384]: https://github.com/actix/actix-net/pull/384
[#2355]: https://github.com/actix/actix-web/pull/2355
[#2356]: https://github.com/actix/actix-web/pull/2356

## 0.5.0-beta.1

- Fix a bug in multi-patterns where static patterns are interpreted as regex. [#366][net#366]
- Introduce `ResourceDef::pattern_iter` to get an iterator over all patterns in a multi-pattern resource. [#373][net#373]
- Fix segment interpolation leaving `Path` in unintended state after matching. [#368][net#368]
- Fix `ResourceDef` `PartialEq` implementation. [#373][net#373]
- Re-work `IntoPatterns` trait, adding a `Patterns` enum. [#372][net#372]
- Implement `IntoPatterns` for `bytestring::ByteString`. [#372][net#372]
- Rename `Path::{len => segment_count}` to be more descriptive of it's purpose. [#370][net#370]
- Rename `ResourceDef::{resource_path => resource_path_from_iter}`. [#371][net#371]
- `ResourceDef::resource_path_from_iter` now takes an `IntoIterator`. [#373][net#373]
- Rename `ResourceDef::{resource_path_named => resource_path_from_map}`. [#371][net#371]
- Rename `ResourceDef::{is_prefix_match => find_match}`. [#373][net#373]
- Rename `ResourceDef::{match_path => capture_match_info}`. [#373][net#373]
- Rename `ResourceDef::{match_path_checked => capture_match_info_fn}`. [#373][net#373]
- Remove `ResourceDef::name_mut` and introduce `ResourceDef::set_name`. [#373][net#373]
- Rename `Router::{*_checked => *_fn}`. [#373][net#373]
- Return type of `ResourceDef::name` is now `Option<&str>`. [#373][net#373]
- Return type of `ResourceDef::pattern` is now `Option<&str>`. [#373][net#373]

[net#368]: https://github.com/actix/actix-net/pull/368
[net#366]: https://github.com/actix/actix-net/pull/366
[net#368]: https://github.com/actix/actix-net/pull/368
[net#370]: https://github.com/actix/actix-net/pull/370
[net#371]: https://github.com/actix/actix-net/pull/371
[net#372]: https://github.com/actix/actix-net/pull/372
[net#373]: https://github.com/actix/actix-net/pull/373

</details>

## 0.4.0

- When matching path parameters, `%25` is now kept in the percent-encoded form; no longer decoded to `%`. [#357][net#357]
- Path tail patterns now match new lines (`\n`) in request URL. [#360][net#360]
- Fixed a safety bug where `Path` could return a malformed string after percent decoding. [#359][net#359]
- Methods `Path::{add, add_static}` now take `impl Into<Cow<'static, str>>`. [#345][net#345]

[net#345]: https://github.com/actix/actix-net/pull/345
[net#357]: https://github.com/actix/actix-net/pull/357
[net#359]: https://github.com/actix/actix-net/pull/359
[net#360]: https://github.com/actix/actix-net/pull/360

## 0.3.0

- Version was yanked previously. See https://crates.io/crates/actix-router/0.3.0

## 0.2.7

- Add `Router::recognize_checked` [#247][net#247]

[net#247]: https://github.com/actix/actix-net/pull/247

## 0.2.6

- Use `bytestring` version range compatible with Bytes v1.0. [#246][net#246]

[net#246]: https://github.com/actix/actix-net/pull/246

## 0.2.5

- Fix `from_hex()` method

## 0.2.4

- Add `ResourceDef::resource_path_named()` path generation method

## 0.2.3

- Add impl `IntoPattern` for `&String`

## 0.2.2

- Use `IntoPattern` for `RouterBuilder::path()`

## 0.2.1

- Add `IntoPattern` trait
- Add multi-pattern resources

## 0.2.0

- Update http to 0.2
- Update regex to 1.3
- Use bytestring instead of string

## 0.1.5

- Remove debug prints

## 0.1.4

- Fix checked resource match

## 0.1.3

- Added support for `remainder match` (i.e "/path/{tail}\*")

## 0.1.2

- Export `Quoter` type
- Allow to reset `Path` instance

## 0.1.1

- Get dynamic segment by name instead of iterator.

## 0.1.0

- Initial release
