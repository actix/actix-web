# Migrating to 3.0.0

- The return type for `ServiceRequest::app_data::<T>()` was changed from returning a `Data<T>` to simply a `T`. To access a `Data<T>` use `ServiceRequest::app_data::<Data<T>>()`.

- Cookie handling has been offloaded to the `cookie` crate:

  - `USERINFO_ENCODE_SET` is no longer exposed. Percent-encoding is still supported; check docs.
  - Some types now require lifetime parameters.

- The time crate was updated to `v0.2`, a major breaking change to the time crate, which affects any `actix-web` method previously expecting a time v0.1 input.

- Setting a cookie's SameSite property, explicitly, to `SameSite::None` will now result in `SameSite=None` being sent with the response Set-Cookie header. To create a cookie without a SameSite attribute, remove any calls setting same_site.

- actix-http support for Actors messages was moved to actix-http crate and is enabled with feature `actors`

- content_length function is removed from actix-http. You can set Content-Length by normally setting the response body or calling no_chunking function.

- `BodySize::Sized64` variant has been removed. `BodySize::Sized` now receives a `u64` instead of a `usize`.

- Code that was using `path.<index>` to access a `web::Path<(A, B, C)>`s elements now needs to use destructuring or `.into_inner()`. For example:

  ```rust
  // Previously:
  async fn some_route(path: web::Path<(String, String)>) -> String {
    format!("Hello, {} {}", path.0, path.1)
  }

  // Now (this also worked before):
  async fn some_route(path: web::Path<(String, String)>) -> String {
    let (first_name, last_name) = path.into_inner();
    format!("Hello, {} {}", first_name, last_name)
  }
  // Or (this wasn't previously supported):
  async fn some_route(web::Path((first_name, last_name)): web::Path<(String, String)>) -> String {
    format!("Hello, {} {}", first_name, last_name)
  }
  ```

- `middleware::NormalizePath` can now also be configured to trim trailing slashes instead of always keeping one. It will need `middleware::normalize::TrailingSlash` when being constructed with `NormalizePath::new(...)`, or for an easier migration you can replace `wrap(middleware::NormalizePath)` with `wrap(middleware::NormalizePath::new(TrailingSlash::MergeOnly))`.

- `HttpServer::maxconn` is renamed to the more expressive `HttpServer::max_connections`.

- `HttpServer::maxconnrate` is renamed to the more expressive `HttpServer::max_connection_rate`.
