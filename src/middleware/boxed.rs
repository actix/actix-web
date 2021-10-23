//! For middleware documentation, see [`Boxed`].

use actix_service::{
    boxed::{self, BoxService},
    Service, Transform,
};
use futures_core::future::LocalBoxFuture;

/// Middleware for boxing another middleware's output. It would do type earse for the final middleware service
/// and reduce type complexity for potential faster compile time in exchange for extra overhead at runtime.
///
/// # Examples
/// ```
/// use actix_web::middleware::{Logger, Boxed};
/// use actix_web::App;
///
/// let app = App::new().wrap(Boxed::new(Logger::default()));
/// ```
pub struct Boxed<T> {
    transform: T,
}

impl<T> Boxed<T> {
    /// Wrap a middleware to erase it's type signature and reduce type complexity.
    pub fn new(middleware: T) -> Self {
        Self {
            transform: middleware,
        }
    }
}

impl<S, T, Req> Transform<S, Req> for Boxed<T>
where
    S: Service<Req> + 'static,
    T: Transform<S, Req> + 'static,
    Req: 'static,
{
    type Response = T::Response;
    type Error = T::Error;
    type Transform = BoxService<Req, T::Response, T::Error>;
    type InitError = T::InitError;
    type Future = LocalBoxFuture<'static, Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let fut = self.transform.new_transform(service);
        Box::pin(async move {
            let service = fut.await?;
            Ok(boxed::service(service))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use actix_service::IntoService;

    use crate::dev::ServiceRequest;
    use crate::http::StatusCode;
    use crate::middleware::Logger;
    use crate::test::{call_service, TestRequest};
    use crate::HttpResponse;

    #[actix_rt::test]
    async fn test_boxed_logger_middleware() {
        let srv = |req: ServiceRequest| {
            Box::pin(async move {
                Ok(req.into_response(HttpResponse::InternalServerError().finish()))
            })
        };

        let mw = Boxed::new(Logger::default())
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
