//! A collection of common middleware.

mod compat;
mod condition;
mod default_headers;
mod err_handlers;
mod logger;
#[cfg(test)]
mod noop;
mod normalize;

pub use self::compat::Compat;
pub use self::condition::Condition;
pub use self::default_headers::DefaultHeaders;
pub use self::err_handlers::{ErrorHandlerResponse, ErrorHandlers};
pub use self::logger::Logger;
#[cfg(test)]
pub(crate) use self::noop::Noop;
pub use self::normalize::{NormalizePath, TrailingSlash};

#[cfg(feature = "__compress")]
mod compress;

#[cfg(feature = "__compress")]
pub use self::compress::Compress;

#[cfg(test)]
mod tests {
    use crate::{http::StatusCode, App};

    use super::*;

    #[test]
    fn common_combinations() {
        // ensure there's no reason that the built-in middleware cannot compose

        let _ = App::new()
            .wrap(Compat::new(Logger::default()))
            .wrap(Condition::new(true, DefaultHeaders::new()))
            .wrap(DefaultHeaders::new().add(("X-Test2", "X-Value2")))
            .wrap(ErrorHandlers::new().handler(StatusCode::FORBIDDEN, |res| {
                Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
            }))
            .wrap(Logger::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim));

        let _ = App::new()
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .wrap(Logger::default())
            .wrap(ErrorHandlers::new().handler(StatusCode::FORBIDDEN, |res| {
                Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
            }))
            .wrap(DefaultHeaders::new().add(("X-Test2", "X-Value2")))
            .wrap(Condition::new(true, DefaultHeaders::new()))
            .wrap(Compat::new(Logger::default()));

        #[cfg(feature = "__compress")]
        {
            let _ = App::new().wrap(Compress::default()).wrap(Logger::default());
            let _ = App::new().wrap(Logger::default()).wrap(Compress::default());
            let _ = App::new().wrap(Compat::new(Compress::default()));
            let _ = App::new().wrap(Condition::new(true, Compat::new(Compress::default())));
        }
    }
}
