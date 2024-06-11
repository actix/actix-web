use actix_http::{header, uri::Uri, RequestHead};

use super::{Guard, GuardContext};

/// Creates a guard that matches requests targeting a specific host.
///
/// # Matching Host
/// This guard will:
/// - match against the `Host` header, if present;
/// - fall-back to matching against the request target's host, if present;
/// - return false if host cannot be determined;
///
/// # Matching Scheme
/// Optionally, this guard can match against the host's scheme. Set the scheme for matching using
/// `Host(host).scheme(protocol)`. If the request's scheme cannot be determined, it will not prevent
/// the guard from matching successfully.
///
/// # Examples
/// The `Host` guard can be used to set up a form of [virtual hosting] within a single app.
/// Overlapping scope prefixes are usually discouraged, but when combined with non-overlapping guard
/// definitions they become safe to use in this way. Without these host guards, only routes under
/// the first-to-be-defined scope would be accessible. You can test this locally using `127.0.0.1`
/// and `localhost` as the `Host` guards.
/// ```
/// use actix_web::{web, http::Method, guard, App, HttpResponse};
///
/// App::new()
///     .service(
///         web::scope("")
///             .guard(guard::Host("www.rust-lang.org"))
///             .default_service(web::to(|| async {
///                 HttpResponse::Ok().body("marketing site")
///             })),
///     )
///     .service(
///         web::scope("")
///             .guard(guard::Host("play.rust-lang.org"))
///             .default_service(web::to(|| async {
///                 HttpResponse::Ok().body("playground frontend")
///             })),
///     );
/// ```
///
/// The example below additionally guards on the host URI's scheme. This could allow routing to
/// different handlers for `http:` vs `https:` visitors; to redirect, for example.
/// ```
/// use actix_web::{web, guard::Host, HttpResponse};
///
/// web::scope("/admin")
///     .guard(Host("admin.rust-lang.org").scheme("https"))
///     .default_service(web::to(|| async {
///         HttpResponse::Ok().body("admin connection is secure")
///     }));
/// ```
///
/// [virtual hosting]: https://en.wikipedia.org/wiki/Virtual_hosting
#[allow(non_snake_case)]
pub fn Host(host: impl AsRef<str>) -> HostGuard {
    HostGuard {
        host: host.as_ref().to_string(),
        scheme: None,
    }
}

fn get_host_uri(req: &RequestHead) -> Option<Uri> {
    req.headers
        .get(header::HOST)
        .and_then(|host_value| host_value.to_str().ok())
        .or_else(|| req.uri.host())
        .and_then(|host| host.parse().ok())
}

#[doc(hidden)]
pub struct HostGuard {
    host: String,
    scheme: Option<String>,
}

impl HostGuard {
    /// Set request scheme to match
    pub fn scheme<H: AsRef<str>>(mut self, scheme: H) -> HostGuard {
        self.scheme = Some(scheme.as_ref().to_string());
        self
    }
}

impl Guard for HostGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        // parse host URI from header or request target
        let req_host_uri = match get_host_uri(ctx.head()) {
            Some(uri) => uri,

            // no match if host cannot be determined
            None => return false,
        };

        match req_host_uri.host() {
            // fall through to scheme checks
            Some(uri_host) if self.host == uri_host => {}

            // Either:
            // - request's host does not match guard's host;
            // - It was possible that the parsed URI from request target did not contain a host.
            _ => return false,
        }

        if let Some(ref scheme) = self.scheme {
            if let Some(ref req_host_uri_scheme) = req_host_uri.scheme_str() {
                return scheme == req_host_uri_scheme;
            }

            // TODO: is this the correct behavior?
            // falls through if scheme cannot be determined
        }

        // all conditions passed
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::TestRequest;

    #[test]
    fn host_from_header() {
        let req = TestRequest::default()
            .insert_header((
                header::HOST,
                header::HeaderValue::from_static("www.rust-lang.org"),
            ))
            .to_srv_request();

        let host = Host("www.rust-lang.org");
        assert!(host.check(&req.guard_ctx()));

        let host = Host("www.rust-lang.org").scheme("https");
        assert!(host.check(&req.guard_ctx()));

        let host = Host("blog.rust-lang.org");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("blog.rust-lang.org").scheme("https");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("crates.io");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("localhost");
        assert!(!host.check(&req.guard_ctx()));
    }

    #[test]
    fn host_without_header() {
        let req = TestRequest::default()
            .uri("www.rust-lang.org")
            .to_srv_request();

        let host = Host("www.rust-lang.org");
        assert!(host.check(&req.guard_ctx()));

        let host = Host("www.rust-lang.org").scheme("https");
        assert!(host.check(&req.guard_ctx()));

        let host = Host("blog.rust-lang.org");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("blog.rust-lang.org").scheme("https");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("crates.io");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("localhost");
        assert!(!host.check(&req.guard_ctx()));
    }

    #[test]
    fn host_scheme() {
        let req = TestRequest::default()
            .insert_header((
                header::HOST,
                header::HeaderValue::from_static("https://www.rust-lang.org"),
            ))
            .to_srv_request();

        let host = Host("www.rust-lang.org").scheme("https");
        assert!(host.check(&req.guard_ctx()));

        let host = Host("www.rust-lang.org");
        assert!(host.check(&req.guard_ctx()));

        let host = Host("www.rust-lang.org").scheme("http");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("blog.rust-lang.org");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("blog.rust-lang.org").scheme("https");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("crates.io").scheme("https");
        assert!(!host.check(&req.guard_ctx()));

        let host = Host("localhost");
        assert!(!host.check(&req.guard_ctx()));
    }
}
