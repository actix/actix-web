# Changes

## Unreleased - 2020-xx-xx


## 2.1.0 - 2020-11-25
* Add ability to set address for `TestServer`. [#1645]
* Upgrade `base64` to `0.13`.
* Upgrade `serde_urlencoded` to `0.7`. [#1773]

[#1773]: https://github.com/actix/actix-web/pull/1773
[#1645]: https://github.com/actix/actix-web/pull/1645


## 2.0.0 - 2020-09-11
* Update actix-codec and actix-utils dependencies.


## 2.0.0-alpha.1 - 2020-05-23
* Update the `time` dependency to 0.2.7
* Update `actix-connect` dependency to 2.0.0-alpha.2
* Make `test_server` `async` fn.
* Bump minimum supported Rust version to 1.40
* Replace deprecated `net2` crate with `socket2`
* Update `base64` dependency to 0.12
* Update `env_logger` dependency to 0.7

## 1.0.0 - 2019-12-13
* Replaced `TestServer::start()` with `test_server()`


## 1.0.0-alpha.3 - 2019-12-07
* Migrate to `std::future`


## 0.2.5 - 2019-09-17
* Update serde_urlencoded to "0.6.1"
* Increase TestServerRuntime timeouts from 500ms to 3000ms
* Do not override current `System`


## 0.2.4 - 2019-07-18
* Update actix-server to 0.6


## 0.2.3 - 2019-07-16
* Add `delete`, `options`, `patch` methods to `TestServerRunner`


## 0.2.2 - 2019-06-16
* Add .put() and .sput() methods


## 0.2.1 - 2019-06-05
* Add license files


## 0.2.0 - 2019-05-12
* Update awc and actix-http deps


## 0.1.1 - 2019-04-24
* Always make new connection for http client


## 0.1.0 - 2019-04-16
* No changes


## 0.1.0-alpha.3 - 2019-04-02
* Request functions accept path #743


## 0.1.0-alpha.2 - 2019-03-29
* Added TestServerRuntime::load_body() method
* Update actix-http and awc libraries


## 0.1.0-alpha.1 - 2019-03-28
* Initial impl
