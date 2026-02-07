# Migrating to 2.0.0

- `HttpServer::start()` renamed to `HttpServer::run()`. It also possible to `.await` on `run` method result, in that case it awaits server exit.

- `App::register_data()` renamed to `App::app_data()` and accepts any type `T: 'static`. Stored data is available via `HttpRequest::app_data()` method at runtime.

- Extractor configuration must be registered with `App::app_data()` instead of `App::data()`

- Sync handlers has been removed. `.to_async()` method has been renamed to `.to()` replace `fn` with `async fn` to convert sync handler to async

- `actix_http_test::TestServer` moved to `actix_web::test` module. To start test server use `test::start()` or `test_start_with_config()` methods

- `ResponseError` trait has been refactored. `ResponseError::error_response()` renders http response.

- Feature `rust-tls` renamed to `rustls`

  instead of

  ```rust
  actix-web = { version = "2.0.0", features = ["rust-tls"] }
  ```

  use

  ```rust
  actix-web = { version = "2.0.0", features = ["rustls"] }
  ```

- Feature `ssl` renamed to `openssl`

  instead of

  ```rust
  actix-web = { version = "2.0.0", features = ["ssl"] }
  ```

  use

  ```rust
  actix-web = { version = "2.0.0", features = ["openssl"] }
  ```

- `Cors` builder now requires that you call `.finish()` to construct the middleware
