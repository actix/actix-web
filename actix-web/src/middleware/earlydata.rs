//! For middleware documentation, see [`Earlydata`].

use crate::{
    body::EitherBody,
    http::{header::HeaderValue, StatusCode},
    service::{ServiceRequest, ServiceResponse},
    Error, HttpResponse,
};
use actix_service::{Service, Transform};
use actix_utils::future::{ready, Ready};
use futures_util::future::LocalBoxFuture;
use std::rc::Rc;

/// The Early Data middleware adds support for TLS 1.3's early data ("0-RTT") feature.
/// Citing [[RFC8446](https://datatracker.ietf.org/doc/html/rfc8446#section-2-3)],
/// when a client and server share a PSK, TLS 1.3 allows clients to send data on the
/// first flight ("early data") to speed up the request, effectively reducing the
/// regular 1-RTT request to a 0-RTT request.
///
/// This 0-RTT request is susceptible to replay attacks, hence it should only be allowed when it's
/// safe to be replayed. By standard, this applies to "safe" HTTP methods. This middleware checks
/// for exactly this and if the used method is not safe, the client is asked to re-perform the
/// request without early data.
///
/// Since the source of the `Early-Data` header has to be trusted, this middleware also allows
/// supplying a function that returns whether the reverse proxy is trusted. If the proxy is not
/// trusted, early data is not allowed.
#[derive(Clone)]
#[non_exhaustive]
pub struct Earlydata {
    /// Function that returns whether the reverse proxy for a given request is trusted.
    is_proxy_trusted: fn(&ServiceRequest) -> bool,
    /// Function that returns whether early data is allowed.
    allow_early_data: fn(&ServiceRequest) -> bool,
}

/// Default function that determines whether early data is allowed. This is accomplished by
/// checking if the method used is safe.
pub fn default_allow_early_data(req: &ServiceRequest) -> bool {
    req.method().is_safe()
}

impl Default for Earlydata {
    /// Returns a default `Earlydata` instance that trusts all proxies and that uses
    /// [`default_allow_early_data`].
    fn default() -> Self {
        Self {
            is_proxy_trusted: |_| -> bool { true },
            allow_early_data: default_allow_early_data,
        }
    }
}

impl Earlydata {
    /// Creates a new `Earlydata` middleware with given functions for determining the behavior.
    pub fn new(
        is_proxy_trusted: fn(&ServiceRequest) -> bool,
        allow_early_data: fn(&ServiceRequest) -> bool,
    ) -> Self {
        Self {
            is_proxy_trusted,
            allow_early_data,
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for Earlydata
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = EarlydataMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(EarlydataMiddleware {
            service: Rc::new(service),
            is_proxy_trusted: self.is_proxy_trusted,
            allow_early_data: self.allow_early_data,
        }))
    }
}

pub struct EarlydataMiddleware<S> {
    service: Rc<S>,
    is_proxy_trusted: fn(&ServiceRequest) -> bool,
    allow_early_data: fn(&ServiceRequest) -> bool,
}

impl<S, B> Service<ServiceRequest> for EarlydataMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        let is_proxy_trusted = self.is_proxy_trusted;
        let allow_early_data = self.allow_early_data;

        Box::pin(async move {
            // Check if this is early data
            // TODO wait for the http PR
            if req.headers().get("early-data") != Some(&HeaderValue::from_static("1")) {
                return service.call(req).await.map(|res| res.map_into_left_body());
            }

            // Do we trust the header?
            if !is_proxy_trusted(&req) {
                return Ok(req.into_response(
                    // TODO wait for PR
                    HttpResponse::new(StatusCode::from_u16(425).unwrap()).map_into_right_body(),
                ));
            }

            if allow_early_data(&req) {
                service.call(req).await.map(|res| res.map_into_left_body())
            } else {
                Ok(req.into_response(
                    // TODO wait for PR
                    HttpResponse::new(StatusCode::from_u16(425).unwrap()).map_into_right_body(),
                ))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{call_service, init_service, TestRequest};
    use crate::{
        http::{header::HeaderValue, Method, StatusCode},
        middleware::earlydata::default_allow_early_data,
        middleware::Earlydata,
        web, App, HttpResponse,
    };

    #[actix_rt::test]
    async fn early_data() {
        let app = init_service(
            App::new()
                .wrap(Earlydata::default())
                .service(web::resource("/").to(HttpResponse::Ok)),
        )
        .await;

        // No early data
        let req = TestRequest::default().uri("/").to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        // Early data (but trusted)
        let req = TestRequest::default()
            .uri("/")
            .insert_header(("early-data", "1"))
            .to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        // Explicitly no early data
        let req = TestRequest::default()
            .uri("/")
            .method(Method::PUT)
            .insert_header(("early-data", "0"))
            .to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        // Early data but PUT
        let req = TestRequest::default()
            .uri("/")
            .method(Method::PUT)
            .insert_header(("early-data", "1"))
            .to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::from_u16(425).unwrap());
    }

    #[actix_rt::test]
    async fn early_data_custom_function() {
        let app = init_service(
            App::new()
                .wrap(Earlydata::new(|_| true, |req| req.method() == Method::PUT))
                .service(web::resource("/").to(HttpResponse::Ok)),
        )
        .await;

        // Should return `true` for PUT now
        let req = TestRequest::default()
            .uri("/")
            .method(Method::PUT)
            .insert_header(("early-data", "1"))
            .to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn early_data_proxy_trust() {
        let app = init_service(
            App::new()
                .wrap(Earlydata::new(
                    |req| {
                        req.headers().get("please-trust-me")
                            == Some(&HeaderValue::from_static("1"))
                    },
                    default_allow_early_data,
                ))
                .service(web::resource("/").to(HttpResponse::Ok)),
        )
        .await;

        // Not trusted -> No 200
        let req = TestRequest::default()
            .uri("/")
            .method(Method::GET)
            .insert_header(("early-data", "1"))
            .to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::from_u16(425).unwrap());

        // Trusted -> 200
        let req = TestRequest::default()
            .uri("/")
            .method(Method::GET)
            .insert_header(("early-data", "1"))
            .insert_header(("please-trust-me", "1"))
            .to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }
}
