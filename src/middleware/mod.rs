//! Commonly used middleware.

mod compat;
mod condition;
mod default_headers;
mod err_handlers;
mod logger;
mod normalize;

pub use self::compat::Compat;
pub use self::condition::conditionally;
pub use self::condition::futurally;
pub use self::condition::optionally;
pub use self::condition::optionally_fut;
pub use self::condition::Condition;
pub use self::default_headers::DefaultHeaders;
pub use self::err_handlers::{ErrorHandlerResponse, ErrorHandlers};
pub use self::logger::Logger;
pub use self::normalize::{NormalizePath, TrailingSlash};

#[cfg(feature = "__compress")]
mod compress;

#[cfg(feature = "__compress")]
pub use self::compress::Compress;

#[cfg(test)]
mod tests {
    use crate::{http::StatusCode, App};

    use super::*;
    use crate::middleware::conditionally;

    #[test]
    fn common_combinations() {
        // ensure there's no reason that the built-in middleware cannot compose

        let _ = App::new()
            .wrap(Compat::new(Logger::default()))
            .wrap(conditionally(true, DefaultHeaders::new()))
            .wrap(DefaultHeaders::new().header("X-Test2", "X-Value2"))
            .wrap(ErrorHandlers::new().handler(StatusCode::FORBIDDEN, |res| {
                Ok(ErrorHandlerResponse::Response(res))
            }))
            .wrap(Logger::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim));

        let _ = App::new()
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .wrap(Logger::default())
            .wrap(ErrorHandlers::new().handler(StatusCode::FORBIDDEN, |res| {
                Ok(ErrorHandlerResponse::Response(res))
            }))
            .wrap(DefaultHeaders::new().header("X-Test2", "X-Value2"))
            .wrap(conditionally(true, DefaultHeaders::new()))
            .wrap(Compat::new(Logger::default()));

        #[cfg(feature = "__compress")]
        {
            let _ = App::new().wrap(Compress::default()).wrap(Logger::default());
            let _ = App::new().wrap(Logger::default()).wrap(Compress::default());
            let _ = App::new().wrap(Compat::new(Compress::default()));
            let _ = App::new().wrap(conditionally(true, Compat::new(Compress::default())));
        }
    }
}
