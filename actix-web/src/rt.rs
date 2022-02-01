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
//! rt::System::new().block_on(
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

// In particular, omit the runtime macros because they won't work and are re-exported directly
// at the top-level anyway. Also omit the `Arbiter` types because they have limited value here.

pub use actix_rt::{net, pin, signal, spawn, task, time, Runtime, System, SystemRunner};
