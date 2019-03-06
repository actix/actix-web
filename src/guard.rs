//! Route match guards.
#![allow(non_snake_case)]
use actix_http::http::{self, header, HttpTryFrom};
use actix_http::RequestHead;

/// Trait defines resource guards. Guards are used for routes selection.
///
/// Guard can not modify request object. But it is possible to
/// to store extra attributes on request by using `Extensions` container,
/// Extensions container available via `RequestHead::extensions()` method.
pub trait Guard {
    /// Check if request matches predicate
    fn check(&self, request: &RequestHead) -> bool;
}

/// Return guard that matches if any of supplied guards.
///
/// ```rust
/// use actix_web::{web, guard, App, HttpResponse};
///
/// fn main() {
///     App::new().service(web::resource("/index.html").route(
///         web::route()
///              .guard(guard::Any(guard::Get()).or(guard::Post()))
///              .to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
pub fn Any<F: Guard + 'static>(guard: F) -> AnyGuard {
    AnyGuard(vec![Box::new(guard)])
}

/// Matches if any of supplied guards matche.
pub struct AnyGuard(Vec<Box<Guard>>);

impl AnyGuard {
    /// Add guard to a list of guards to check
    pub fn or<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.0.push(Box::new(guard));
        self
    }
}

impl Guard for AnyGuard {
    fn check(&self, req: &RequestHead) -> bool {
        for p in &self.0 {
            if p.check(req) {
                return true;
            }
        }
        false
    }
}

/// Return guard that matches if all of the supplied guards.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{guard, web, App, HttpResponse};
///
/// fn main() {
///     App::new().service(web::resource("/index.html").route(
///         web::route()
///             .guard(
///                 guard::All(guard::Get()).and(guard::Header("content-type", "text/plain")))
///             .to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
pub fn All<F: Guard + 'static>(guard: F) -> AllGuard {
    AllGuard(vec![Box::new(guard)])
}

/// Matches if all of supplied guards.
pub struct AllGuard(Vec<Box<Guard>>);

impl AllGuard {
    /// Add new guard to the list of guards to check
    pub fn and<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.0.push(Box::new(guard));
        self
    }
}

impl Guard for AllGuard {
    fn check(&self, request: &RequestHead) -> bool {
        for p in &self.0 {
            if !p.check(request) {
                return false;
            }
        }
        true
    }
}

/// Return guard that matches if supplied guard does not match.
pub fn Not<F: Guard + 'static>(guard: F) -> NotGuard {
    NotGuard(Box::new(guard))
}

#[doc(hidden)]
pub struct NotGuard(Box<Guard>);

impl Guard for NotGuard {
    fn check(&self, request: &RequestHead) -> bool {
        !self.0.check(request)
    }
}

/// Http method guard
#[doc(hidden)]
pub struct MethodGuard(http::Method);

impl Guard for MethodGuard {
    fn check(&self, request: &RequestHead) -> bool {
        request.method == self.0
    }
}

/// Guard to match *GET* http method
pub fn Get() -> MethodGuard {
    MethodGuard(http::Method::GET)
}

/// Predicate to match *POST* http method
pub fn Post() -> MethodGuard {
    MethodGuard(http::Method::POST)
}

/// Predicate to match *PUT* http method
pub fn Put() -> MethodGuard {
    MethodGuard(http::Method::PUT)
}

/// Predicate to match *DELETE* http method
pub fn Delete() -> MethodGuard {
    MethodGuard(http::Method::DELETE)
}

/// Predicate to match *HEAD* http method
pub fn Head() -> MethodGuard {
    MethodGuard(http::Method::HEAD)
}

/// Predicate to match *OPTIONS* http method
pub fn Options() -> MethodGuard {
    MethodGuard(http::Method::OPTIONS)
}

/// Predicate to match *CONNECT* http method
pub fn Connect() -> MethodGuard {
    MethodGuard(http::Method::CONNECT)
}

/// Predicate to match *PATCH* http method
pub fn Patch() -> MethodGuard {
    MethodGuard(http::Method::PATCH)
}

/// Predicate to match *TRACE* http method
pub fn Trace() -> MethodGuard {
    MethodGuard(http::Method::TRACE)
}

/// Predicate to match specified http method
pub fn Method(method: http::Method) -> MethodGuard {
    MethodGuard(method)
}

/// Return predicate that matches if request contains specified header and
/// value.
pub fn Header(name: &'static str, value: &'static str) -> HeaderGuard {
    HeaderGuard(
        header::HeaderName::try_from(name).unwrap(),
        header::HeaderValue::from_static(value),
    )
}

#[doc(hidden)]
pub struct HeaderGuard(header::HeaderName, header::HeaderValue);

impl Guard for HeaderGuard {
    fn check(&self, req: &RequestHead) -> bool {
        if let Some(val) = req.headers.get(&self.0) {
            return val == self.1;
        }
        false
    }
}

// /// Return predicate that matches if request contains specified Host name.
// ///
// /// ```rust,ignore
// /// # extern crate actix_web;
// /// use actix_web::{pred, App, HttpResponse};
// ///
// /// fn main() {
// ///     App::new().resource("/index.html", |r| {
// ///         r.route()
// ///             .guard(pred::Host("www.rust-lang.org"))
// ///             .f(|_| HttpResponse::MethodNotAllowed())
// ///     });
// /// }
// /// ```
// pub fn Host<H: AsRef<str>>(host: H) -> HostGuard {
//     HostGuard(host.as_ref().to_string(), None)
// }

// #[doc(hidden)]
// pub struct HostGuard(String, Option<String>);

// impl HostGuard {
//     /// Set reuest scheme to match
//     pub fn scheme<H: AsRef<str>>(&mut self, scheme: H) {
//         self.1 = Some(scheme.as_ref().to_string())
//     }
// }

// impl Guard for HostGuard {
//     fn check(&self, _req: &RequestHead) -> bool {
//         // let info = req.connection_info();
//         // if let Some(ref scheme) = self.1 {
//         //     self.0 == info.host() && scheme == info.scheme()
//         // } else {
//         //     self.0 == info.host()
//         // }
//         false
//     }
// }

#[cfg(test)]
mod tests {
    use actix_http::http::{header, Method};

    use super::*;
    use crate::test::TestRequest;

    #[test]
    fn test_header() {
        let req = TestRequest::with_header(header::TRANSFER_ENCODING, "chunked")
            .to_http_request();

        let pred = Header("transfer-encoding", "chunked");
        assert!(pred.check(&req));

        let pred = Header("transfer-encoding", "other");
        assert!(!pred.check(&req));

        let pred = Header("content-type", "other");
        assert!(!pred.check(&req));
    }

    // #[test]
    // fn test_host() {
    //     let req = TestServiceRequest::default()
    //         .header(
    //             header::HOST,
    //             header::HeaderValue::from_static("www.rust-lang.org"),
    //         )
    //         .request();

    //     let pred = Host("www.rust-lang.org");
    //     assert!(pred.check(&req));

    //     let pred = Host("localhost");
    //     assert!(!pred.check(&req));
    // }

    #[test]
    fn test_methods() {
        let req = TestRequest::default().to_http_request();
        let req2 = TestRequest::default()
            .method(Method::POST)
            .to_http_request();

        assert!(Get().check(&req));
        assert!(!Get().check(&req2));
        assert!(Post().check(&req2));
        assert!(!Post().check(&req));

        let r = TestRequest::default().method(Method::PUT).to_http_request();
        assert!(Put().check(&r));
        assert!(!Put().check(&req));

        let r = TestRequest::default()
            .method(Method::DELETE)
            .to_http_request();
        assert!(Delete().check(&r));
        assert!(!Delete().check(&req));

        let r = TestRequest::default()
            .method(Method::HEAD)
            .to_http_request();
        assert!(Head().check(&r));
        assert!(!Head().check(&req));

        let r = TestRequest::default()
            .method(Method::OPTIONS)
            .to_http_request();
        assert!(Options().check(&r));
        assert!(!Options().check(&req));

        let r = TestRequest::default()
            .method(Method::CONNECT)
            .to_http_request();
        assert!(Connect().check(&r));
        assert!(!Connect().check(&req));

        let r = TestRequest::default()
            .method(Method::PATCH)
            .to_http_request();
        assert!(Patch().check(&r));
        assert!(!Patch().check(&req));

        let r = TestRequest::default()
            .method(Method::TRACE)
            .to_http_request();
        assert!(Trace().check(&r));
        assert!(!Trace().check(&req));
    }

    #[test]
    fn test_preds() {
        let r = TestRequest::default()
            .method(Method::TRACE)
            .to_http_request();

        assert!(Not(Get()).check(&r));
        assert!(!Not(Trace()).check(&r));

        assert!(All(Trace()).and(Trace()).check(&r));
        assert!(!All(Get()).and(Trace()).check(&r));

        assert!(Any(Get()).or(Trace()).check(&r));
        assert!(!Any(Get()).or(Get()).check(&r));
    }
}
