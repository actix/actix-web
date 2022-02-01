//! A selection of re-exports from [`actix-rt`] and [`tokio`].
//!
//! [`actix-rt`]: https://docs.rs/actix_rt
//! [`tokio`]: https://docs.rs/tokio
//!
//! # Running Actix Web Macro-less
//! ```no_run
//! use actix_web::{middleware, rt, web, App, HttpRequest, HttpServer};
//!
//! async fn index(req: HttpRequest) -> &'static str {
//!     println!("REQ: {:?}", req);
//!     "Hello world!\r\n"
//! }
//!
//! # fn main() -> std::io::Result<()> {
//! rt::block_on(
//!     HttpServer::new(|| {
//!         App::new()
//!             .wrap(middleware::Logger::default())
//!             .service(web::resource("/").route(web::get().to(index)))
//!     })
//!     .bind(("127.0.0.1", 8080))?
//!     .workers(1)
//!     .run()
//! )
//! # }
//! ```

// In particular:
// - Omit the `Arbiter` types because they have limited value here.
// - Re-export but hide the runtime macros because they won't work directly but are required for
//   `#[actix_web::main]` and `#[actix_web::test]` to work.

pub use actix_rt::{net, pin, signal, spawn, task, time, Runtime, System, SystemRunner};

#[cfg(feature = "macros")]
#[doc(hidden)]
pub use actix_rt::{main, test};
