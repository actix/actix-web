//! See [`Redirect`] for service/responder documentation.

use std::borrow::Cow;

use actix_utils::future::ready;

use crate::{
    dev::{fn_service, AppService, HttpServiceFactory, ResourceDef, ServiceRequest},
    http::{header::LOCATION, StatusCode},
    HttpRequest, HttpResponse, Responder,
};

/// An HTTP service for redirecting one path to another path or URL.
///
/// By default, the "307 Temporary Redirect" status is used when responding. See [this MDN
/// article][mdn-redirects] on why 307 is preferred over 302.
///
/// # Examples
/// As service:
/// ```
/// use actix_web::{web, App};
///
/// App::new()
///     // redirect "/duck" to DuckDuckGo
///     .service(web::redirect("/duck", "https://duck.com"))
///     .service(
///         // redirect "/api/old" to "/api/new"
///         web::scope("/api").service(web::redirect("/old", "/new"))
///     );
/// ```
///
/// As responder:
/// ```
/// use actix_web::{web::Redirect, Responder};
///
/// async fn handler() -> impl Responder {
///     // sends a permanent (308) redirect to duck.com
///     Redirect::to("https://duck.com").permanent()
/// }
/// # actix_web::web::to(handler);
/// ```
///
/// [mdn-redirects]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Redirections#temporary_redirections
#[derive(Debug, Clone)]
pub struct Redirect {
    from: Cow<'static, str>,
    to: Cow<'static, str>,
    status_code: StatusCode,
}

impl Redirect {
    /// Construct a new `Redirect` service that matches a path.
    ///
    /// This service will match exact paths equal to `from` within the current scope. I.e., when
    /// registered on the root `App`, it will match exact, whole paths. But when registered on a
    /// `Scope`, it will match paths under that scope, ignoring the defined scope prefix, just like
    /// a normal `Resource` or `Route`.
    ///
    /// The `to` argument can be path or URL; whatever is provided shall be used verbatim when
    /// setting the redirect location. This means that relative paths can be used to navigate
    /// relatively to matched paths.
    ///
    /// Prefer [`Redirect::to()`](Self::to) when using `Redirect` as a responder since `from` has
    /// no meaning in that context.
    ///
    /// # Examples
    /// ```
    /// # use actix_web::{web::Redirect, App};
    /// App::new()
    ///     // redirects "/oh/hi/mark" to "/oh/bye/johnny"
    ///     .service(Redirect::new("/oh/hi/mark", "../../bye/johnny"));
    /// ```
    pub fn new(from: impl Into<Cow<'static, str>>, to: impl Into<Cow<'static, str>>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            status_code: StatusCode::TEMPORARY_REDIRECT,
        }
    }

    /// Construct a new `Redirect` to use as a responder.
    ///
    /// Only receives the `to` argument since responders do not need to do route matching.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{web::Redirect, Responder};
    ///
    /// async fn admin_page() -> impl Responder {
    ///     // sends a temporary 307 redirect to the login path
    ///     Redirect::to("/login")
    /// }
    /// # actix_web::web::to(admin_page);
    /// ```
    pub fn to(to: impl Into<Cow<'static, str>>) -> Self {
        Self {
            from: "/".into(),
            to: to.into(),
            status_code: StatusCode::TEMPORARY_REDIRECT,
        }
    }

    /// Use the "308 Permanent Redirect" status when responding.
    ///
    /// See [this MDN article][mdn-redirects] on why 308 is preferred over 301.
    ///
    /// [mdn-redirects]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Redirections#permanent_redirections
    pub fn permanent(self) -> Self {
        self.using_status_code(StatusCode::PERMANENT_REDIRECT)
    }

    /// Use the "307 Temporary Redirect" status when responding.
    ///
    /// See [this MDN article][mdn-redirects] on why 307 is preferred over 302.
    ///
    /// [mdn-redirects]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Redirections#temporary_redirections
    pub fn temporary(self) -> Self {
        self.using_status_code(StatusCode::TEMPORARY_REDIRECT)
    }

    /// Use the "303 See Other" status when responding.
    ///
    /// This status code is semantically correct as the response to a successful login, for example.
    pub fn see_other(self) -> Self {
        self.using_status_code(StatusCode::SEE_OTHER)
    }

    /// Allows the use of custom status codes for less common redirect types.
    ///
    /// In most cases, the default status ("308 Permanent Redirect") or using the `temporary`
    /// method, which uses the "307 Temporary Redirect" status have more consistent behavior than
    /// 301 and 302 codes, respectively.
    ///
    /// ```
    /// # use actix_web::{http::StatusCode, web::Redirect};
    /// // redirects would use "301 Moved Permanently" status code
    /// Redirect::new("/old", "/new")
    ///     .using_status_code(StatusCode::MOVED_PERMANENTLY);
    ///
    /// // redirects would use "302 Found" status code
    /// Redirect::new("/old", "/new")
    ///     .using_status_code(StatusCode::FOUND);
    /// ```
    pub fn using_status_code(mut self, status: StatusCode) -> Self {
        self.status_code = status;
        self
    }
}

impl HttpServiceFactory for Redirect {
    fn register(self, config: &mut AppService) {
        let redirect = self.clone();
        let rdef = ResourceDef::new(self.from.into_owned());
        let redirect_factory = fn_service(move |mut req: ServiceRequest| {
            let res = redirect.clone().respond_to(req.parts_mut().0);
            ready(Ok(req.into_response(res.map_into_boxed_body())))
        });

        config.register_service(rdef, None, redirect_factory, None)
    }
}

impl Responder for Redirect {
    type Body = ();

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        let mut res = HttpResponse::with_body(self.status_code, ());

        if let Ok(hdr_val) = self.to.parse() {
            res.headers_mut().insert(LOCATION, hdr_val);
        } else {
            log::error!(
                "redirect target location can not be converted to header value: {:?}",
                self.to,
            );
        }

        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dev::Service, test, App};

    #[actix_rt::test]
    async fn absolute_redirects() {
        let redirector = Redirect::new("/one", "/two").permanent();

        let svc = test::init_service(App::new().service(redirector)).await;

        let req = test::TestRequest::default().uri("/one").to_request();
        let res = svc.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::from_u16(308).unwrap());
        let hdr = res.headers().get(&LOCATION).unwrap();
        assert_eq!(hdr.to_str().unwrap(), "/two");
    }

    #[actix_rt::test]
    async fn relative_redirects() {
        let redirector = Redirect::new("/one", "two").permanent();

        let svc = test::init_service(App::new().service(redirector)).await;

        let req = test::TestRequest::default().uri("/one").to_request();
        let res = svc.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::from_u16(308).unwrap());
        let hdr = res.headers().get(&LOCATION).unwrap();
        assert_eq!(hdr.to_str().unwrap(), "two");
    }

    #[actix_rt::test]
    async fn temporary_redirects() {
        let external_service = Redirect::new("/external", "https://duck.com");

        let svc = test::init_service(App::new().service(external_service)).await;

        let req = test::TestRequest::default().uri("/external").to_request();
        let res = svc.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::from_u16(307).unwrap());
        let hdr = res.headers().get(&LOCATION).unwrap();
        assert_eq!(hdr.to_str().unwrap(), "https://duck.com");
    }

    #[actix_rt::test]
    async fn as_responder() {
        let responder = Redirect::to("https://duck.com");

        let req = test::TestRequest::default().to_http_request();
        let res = responder.respond_to(&req);

        assert_eq!(res.status(), StatusCode::from_u16(307).unwrap());
        let hdr = res.headers().get(&LOCATION).unwrap();
        assert_eq!(hdr.to_str().unwrap(), "https://duck.com");
    }
}
