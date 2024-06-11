//! A selection of re-exports from [`tokio`] and [`actix-rt`].
//!
//! Actix Web runs on [Tokio], providing full[^compat] compatibility with its huge ecosystem of
//! crates. Each of the server's workers uses a single-threaded runtime. Read more about the
//! architecture in [`actix-rt`]'s docs.
//!
//! # Running Actix Web Without Macros
//!
//! ```no_run
//! use actix_web::{middleware, rt, web, App, HttpRequest, HttpServer};
//!
//! async fn index(req: HttpRequest) -> &'static str {
//!     println!("REQ: {:?}", req);
//!     "Hello world!\r\n"
//! }
//!
//! fn main() -> std::io::Result<()> {
//!     rt::System::new().block_on(
//!         HttpServer::new(|| {
//!             App::new().service(web::resource("/").route(web::get().to(index)))
//!         })
//!         .bind(("127.0.0.1", 8080))?
//!         .run()
//!     )
//! }
//! ```
//!
//! # Running Actix Web Using `#[tokio::main]`
//!
//! If you need to run something that uses Tokio's work stealing functionality alongside Actix Web,
//! you can run Actix Web under `#[tokio::main]`. The [`Server`](crate::dev::Server) object returned
//! from [`HttpServer::run`](crate::HttpServer::run) can also be [`spawn`]ed, if preferred.
//!
//! Note that `actix` actor support (and therefore WebSocket support through `actix-web-actors`)
//! still require `#[actix_web::main]` since they require a [`System`] to be set up.
//!
//! Also note that calls to this module's [`spawn()`] re-export require an `#[actix_web::main]`
//! runtime (or a manually configured `LocalSet`) since it makes calls into to the current thread's
//! `LocalSet`, which `#[tokio::main]` does not set up.
//!
//! ```no_run
//! use actix_web::{get, middleware, rt, web, App, HttpRequest, HttpServer};
//!
//! #[get("/")]
//! async fn index(req: HttpRequest) -> &'static str {
//!     println!("REQ: {:?}", req);
//!     "Hello world!\r\n"
//! }
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     HttpServer::new(|| {
//!         App::new().service(index)
//!     })
//!     .bind(("127.0.0.1", 8080))?
//!     .run()
//!     .await
//! }
//! ```
//!
//! [^compat]: Crates that use Tokio's [`block_in_place`] will not work with Actix Web. Fortunately,
//!   the vast majority of Tokio-based crates do not use it.
//!
//! [`actix-rt`]: https://docs.rs/actix-rt
//! [`tokio`]: https://docs.rs/tokio
//! [Tokio]: https://docs.rs/tokio
//! [`spawn`]: https://docs.rs/tokio/1/tokio/fn.spawn.html
//! [`block_in_place`]: https://docs.rs/tokio/1/tokio/task/fn.block_in_place.html

// In particular:
// - Omit the `Arbiter` types because they have limited value here.
// - Re-export but hide the runtime macros because they won't work directly but are required for
//   `#[actix_web::main]` and `#[actix_web::test]` to work.

#[cfg(feature = "macros")]
#[doc(hidden)]
pub use actix_macros::{main, test};
pub use actix_rt::{net, pin, signal, spawn, task, time, Runtime, System, SystemRunner};
