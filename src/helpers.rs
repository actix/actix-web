//! Various helpers

use http::{header, StatusCode};
use regex::Regex;

use handler::Handler;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Path normalization helper
///
/// By normalizing it means:
///
/// - Add a trailing slash to the path.
/// - Remove a trailing slash from the path.
/// - Double slashes are replaced by one.
///
/// The handler returns as soon as it finds a path that resolves
/// correctly. The order if all enable is 1) merge, 3) both merge and append
/// and 3) append. If the path resolves with
/// at least one of those conditions, it will redirect to the new path.
///
/// If *append* is *true* append slash when needed. If a resource is
/// defined with trailing slash and the request comes without it, it will
/// append it automatically.
///
/// If *merge* is *true*, merge multiple consecutive slashes in the path into
/// one.
///
/// This handler designed to be use as a handler for application's *default
/// resource*.
///
/// ```rust
/// # extern crate actix_web;
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// use actix_web::http::NormalizePath;
///
/// # fn index(req: HttpRequest) -> HttpResponse {
/// #     HttpResponse::Ok().into()
/// # }
/// fn main() {
///     let app = App::new()
///         .resource("/test/", |r| r.f(index))
///         .default_resource(|r| r.h(NormalizePath::default()))
///         .finish();
/// }
/// ```
/// In this example `/test`, `/test///` will be redirected to `/test/` url.
pub struct NormalizePath {
    append: bool,
    merge: bool,
    re_merge: Regex,
    redirect: StatusCode,
    not_found: StatusCode,
}

impl Default for NormalizePath {
    /// Create default `NormalizePath` instance, *append* is set to *true*,
    /// *merge* is set to *true* and *redirect* is set to
    /// `StatusCode::MOVED_PERMANENTLY`
    fn default() -> NormalizePath {
        NormalizePath {
            append: true,
            merge: true,
            re_merge: Regex::new("//+").unwrap(),
            redirect: StatusCode::MOVED_PERMANENTLY,
            not_found: StatusCode::NOT_FOUND,
        }
    }
}

impl NormalizePath {
    /// Create new `NormalizePath` instance
    pub fn new(append: bool, merge: bool, redirect: StatusCode) -> NormalizePath {
        NormalizePath {
            append,
            merge,
            redirect,
            re_merge: Regex::new("//+").unwrap(),
            not_found: StatusCode::NOT_FOUND,
        }
    }
}

impl<S> Handler<S> for NormalizePath {
    type Result = HttpResponse;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        if let Some(router) = req.router() {
            let query = req.query_string();
            if self.merge {
                // merge slashes
                let p = self.re_merge.replace_all(req.path(), "/");
                if p.len() != req.path().len() {
                    if router.has_route(p.as_ref()) {
                        let p = if !query.is_empty() {
                            p + "?" + query
                        } else {
                            p
                        };
                        return HttpResponse::build(self.redirect)
                            .header(header::LOCATION, p.as_ref())
                            .finish();
                    }
                    // merge slashes and append trailing slash
                    if self.append && !p.ends_with('/') {
                        let p = p.as_ref().to_owned() + "/";
                        if router.has_route(&p) {
                            let p = if !query.is_empty() {
                                p + "?" + query
                            } else {
                                p
                            };
                            return HttpResponse::build(self.redirect)
                                .header(header::LOCATION, p.as_str())
                                .finish();
                        }
                    }

                    // try to remove trailing slash
                    if p.ends_with('/') {
                        let p = p.as_ref().trim_right_matches('/');
                        if router.has_route(p) {
                            let mut req = HttpResponse::build(self.redirect);
                            return if !query.is_empty() {
                                req.header(
                                    header::LOCATION,
                                    (p.to_owned() + "?" + query).as_str(),
                                )
                            } else {
                                req.header(header::LOCATION, p)
                            }.finish();
                        }
                    }
                } else if p.ends_with('/') {
                    // try to remove trailing slash
                    let p = p.as_ref().trim_right_matches('/');
                    if router.has_route(p) {
                        let mut req = HttpResponse::build(self.redirect);
                        return if !query.is_empty() {
                            req.header(
                                header::LOCATION,
                                (p.to_owned() + "?" + query).as_str(),
                            )
                        } else {
                            req.header(header::LOCATION, p)
                        }.finish();
                    }
                }
            }
            // append trailing slash
            if self.append && !req.path().ends_with('/') {
                let p = req.path().to_owned() + "/";
                if router.has_route(&p) {
                    let p = if !query.is_empty() {
                        p + "?" + query
                    } else {
                        p
                    };
                    return HttpResponse::build(self.redirect)
                        .header(header::LOCATION, p.as_str())
                        .finish();
                }
            }
        }
        HttpResponse::new(self.not_found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::App;
    use http::{header, Method};
    use test::TestRequest;

    fn index(_req: HttpRequest) -> HttpResponse {
        HttpResponse::new(StatusCode::OK)
    }

    #[test]
    fn test_normalize_path_trailing_slashes() {
        let mut app = App::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1", "", StatusCode::OK),
            (
                "/resource1/",
                "/resource1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/resource2",
                "/resource2/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            ("/resource2/", "", StatusCode::OK),
            ("/resource1?p1=1&p2=2", "", StatusCode::OK),
            (
                "/resource1/?p1=1&p2=2",
                "/resource1?p1=1&p2=2",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/resource2?p1=1&p2=2",
                "/resource2/?p1=1&p2=2",
                StatusCode::MOVED_PERMANENTLY,
            ),
            ("/resource2/?p1=1&p2=2", "", StatusCode::OK),
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_msg();
            assert_eq!(r.status(), code);
            if !target.is_empty() {
                assert_eq!(
                    target,
                    r.headers()
                        .get(header::LOCATION)
                        .unwrap()
                        .to_str()
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn test_normalize_path_trailing_slashes_disabled() {
        let mut app = App::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| {
                r.h(NormalizePath::new(
                    false,
                    true,
                    StatusCode::MOVED_PERMANENTLY,
                ))
            })
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1", StatusCode::OK),
            ("/resource1/", StatusCode::MOVED_PERMANENTLY),
            ("/resource2", StatusCode::NOT_FOUND),
            ("/resource2/", StatusCode::OK),
            ("/resource1?p1=1&p2=2", StatusCode::OK),
            ("/resource1/?p1=1&p2=2", StatusCode::MOVED_PERMANENTLY),
            ("/resource2?p1=1&p2=2", StatusCode::NOT_FOUND),
            ("/resource2/?p1=1&p2=2", StatusCode::OK),
        ];
        for (path, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_msg();
            assert_eq!(r.status(), code);
        }
    }

    #[test]
    fn test_normalize_path_merge_slashes() {
        let mut app = App::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource1/a/b", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1/a/b", "", StatusCode::OK),
            (
                "/resource1/",
                "/resource1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/resource1//",
                "/resource1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource1//a//b",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource1//a//b/",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource1//a//b//",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource1//a//b",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a///b",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a//b/",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            ("/resource1/a/b?p=1", "", StatusCode::OK),
            (
                "//resource1//a//b?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource1//a//b/?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource1//a//b?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a///b?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a//b/?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a//b//?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_msg();
            assert_eq!(r.status(), code);
            if !target.is_empty() {
                assert_eq!(
                    target,
                    r.headers()
                        .get(header::LOCATION)
                        .unwrap()
                        .to_str()
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn test_normalize_path_merge_and_append_slashes() {
        let mut app = App::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .resource("/resource1/a/b", |r| r.method(Method::GET).f(index))
            .resource("/resource2/a/b/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1/a/b", "", StatusCode::OK),
            (
                "/resource1/a/b/",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b/",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b//",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource1//a//b",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource1//a//b/",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a///b",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a///b/",
                "/resource1/a/b",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/resource2/a/b",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            ("/resource2/a/b/", "", StatusCode::OK),
            (
                "//resource2//a//b",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b/",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource2//a//b",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource2//a//b/",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource2/a///b",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource2/a///b/",
                "/resource2/a/b/",
                StatusCode::MOVED_PERMANENTLY,
            ),
            ("/resource1/a/b?p=1", "", StatusCode::OK),
            (
                "/resource1/a/b/?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b/?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource1//a//b?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource1//a//b/?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a///b?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a///b/?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource1/a///b//?p=1",
                "/resource1/a/b?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/resource2/a/b?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "//resource2//a//b/?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource2//a//b?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "///resource2//a//b/?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource2/a///b?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
            (
                "/////resource2/a///b/?p=1",
                "/resource2/a/b/?p=1",
                StatusCode::MOVED_PERMANENTLY,
            ),
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_msg();
            assert_eq!(r.status(), code);
            if !target.is_empty() {
                assert_eq!(
                    target,
                    r.headers()
                        .get(header::LOCATION)
                        .unwrap()
                        .to_str()
                        .unwrap()
                );
            }
        }
    }
}
