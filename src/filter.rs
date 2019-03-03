//! Route match predicates
#![allow(non_snake_case)]
use actix_http::http::{self, header, HttpTryFrom};
use actix_http::RequestHead;

/// Trait defines resource predicate.
/// Predicate can modify request object. It is also possible to
/// to store extra attributes on request by using `Extensions` container,
/// Extensions container available via `HttpRequest::extensions()` method.
pub trait Filter {
    /// Check if request matches predicate
    fn check(&self, request: &RequestHead) -> bool;
}

/// Return filter that matches if any of supplied filters.
///
/// ```rust
/// use actix_web::{web, filter, App, HttpResponse};
///
/// fn main() {
///     App::new().resource("/index.html", |r|
///         r.route(
///             web::route()
///                  .filter(filter::Any(filter::Get()).or(filter::Post()))
///                  .to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
pub fn Any<F: Filter + 'static>(filter: F) -> AnyFilter {
    AnyFilter(vec![Box::new(filter)])
}

/// Matches if any of supplied filters matche.
pub struct AnyFilter(Vec<Box<Filter>>);

impl AnyFilter {
    /// Add filter to a list of filters to check
    pub fn or<F: Filter + 'static>(mut self, filter: F) -> Self {
        self.0.push(Box::new(filter));
        self
    }
}

impl Filter for AnyFilter {
    fn check(&self, req: &RequestHead) -> bool {
        for p in &self.0 {
            if p.check(req) {
                return true;
            }
        }
        false
    }
}

/// Return filter that matches if all of supplied filters match.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{filter, web, App, HttpResponse};
///
/// fn main() {
///     App::new().resource("/index.html", |r| {
///         r.route(web::route()
///             .filter(
///                 filter::All(filter::Get()).and(filter::Header("content-type", "text/plain")))
///             .to(|| HttpResponse::MethodNotAllowed()))
///     });
/// }
/// ```
pub fn All<F: Filter + 'static>(filter: F) -> AllFilter {
    AllFilter(vec![Box::new(filter)])
}

/// Matches if all of supplied filters matche.
pub struct AllFilter(Vec<Box<Filter>>);

impl AllFilter {
    /// Add new predicate to list of predicates to check
    pub fn and<F: Filter + 'static>(mut self, filter: F) -> Self {
        self.0.push(Box::new(filter));
        self
    }
}

impl Filter for AllFilter {
    fn check(&self, request: &RequestHead) -> bool {
        for p in &self.0 {
            if !p.check(request) {
                return false;
            }
        }
        true
    }
}

/// Return predicate that matches if supplied predicate does not match.
pub fn Not<F: Filter + 'static>(filter: F) -> NotFilter {
    NotFilter(Box::new(filter))
}

#[doc(hidden)]
pub struct NotFilter(Box<Filter>);

impl Filter for NotFilter {
    fn check(&self, request: &RequestHead) -> bool {
        !self.0.check(request)
    }
}

/// Http method predicate
#[doc(hidden)]
pub struct MethodFilter(http::Method);

impl Filter for MethodFilter {
    fn check(&self, request: &RequestHead) -> bool {
        request.method == self.0
    }
}

/// Predicate to match *GET* http method
pub fn Get() -> MethodFilter {
    MethodFilter(http::Method::GET)
}

/// Predicate to match *POST* http method
pub fn Post() -> MethodFilter {
    MethodFilter(http::Method::POST)
}

/// Predicate to match *PUT* http method
pub fn Put() -> MethodFilter {
    MethodFilter(http::Method::PUT)
}

/// Predicate to match *DELETE* http method
pub fn Delete() -> MethodFilter {
    MethodFilter(http::Method::DELETE)
}

/// Predicate to match *HEAD* http method
pub fn Head() -> MethodFilter {
    MethodFilter(http::Method::HEAD)
}

/// Predicate to match *OPTIONS* http method
pub fn Options() -> MethodFilter {
    MethodFilter(http::Method::OPTIONS)
}

/// Predicate to match *CONNECT* http method
pub fn Connect() -> MethodFilter {
    MethodFilter(http::Method::CONNECT)
}

/// Predicate to match *PATCH* http method
pub fn Patch() -> MethodFilter {
    MethodFilter(http::Method::PATCH)
}

/// Predicate to match *TRACE* http method
pub fn Trace() -> MethodFilter {
    MethodFilter(http::Method::TRACE)
}

/// Predicate to match specified http method
pub fn Method(method: http::Method) -> MethodFilter {
    MethodFilter(method)
}

/// Return predicate that matches if request contains specified header and
/// value.
pub fn Header(name: &'static str, value: &'static str) -> HeaderFilter {
    HeaderFilter(
        header::HeaderName::try_from(name).unwrap(),
        header::HeaderValue::from_static(value),
    )
}

#[doc(hidden)]
pub struct HeaderFilter(header::HeaderName, header::HeaderValue);

impl Filter for HeaderFilter {
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
// ///             .filter(pred::Host("www.rust-lang.org"))
// ///             .f(|_| HttpResponse::MethodNotAllowed())
// ///     });
// /// }
// /// ```
// pub fn Host<H: AsRef<str>>(host: H) -> HostFilter {
//     HostFilter(host.as_ref().to_string(), None)
// }

// #[doc(hidden)]
// pub struct HostFilter(String, Option<String>);

// impl HostFilter {
//     /// Set reuest scheme to match
//     pub fn scheme<H: AsRef<str>>(&mut self, scheme: H) {
//         self.1 = Some(scheme.as_ref().to_string())
//     }
// }

// impl Filter for HostFilter {
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
            .finish()
            .into_request();

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
        let req = TestRequest::default().finish().into_request();
        let req2 = TestRequest::default()
            .method(Method::POST)
            .finish()
            .into_request();

        assert!(Get().check(&req));
        assert!(!Get().check(&req2));
        assert!(Post().check(&req2));
        assert!(!Post().check(&req));

        let r = TestRequest::default().method(Method::PUT).finish();
        assert!(Put().check(&r,));
        assert!(!Put().check(&req,));

        let r = TestRequest::default().method(Method::DELETE).finish();
        assert!(Delete().check(&r,));
        assert!(!Delete().check(&req,));

        let r = TestRequest::default().method(Method::HEAD).finish();
        assert!(Head().check(&r,));
        assert!(!Head().check(&req,));

        let r = TestRequest::default().method(Method::OPTIONS).finish();
        assert!(Options().check(&r,));
        assert!(!Options().check(&req,));

        let r = TestRequest::default().method(Method::CONNECT).finish();
        assert!(Connect().check(&r,));
        assert!(!Connect().check(&req,));

        let r = TestRequest::default().method(Method::PATCH).finish();
        assert!(Patch().check(&r,));
        assert!(!Patch().check(&req,));

        let r = TestRequest::default().method(Method::TRACE).finish();
        assert!(Trace().check(&r,));
        assert!(!Trace().check(&req,));
    }

    #[test]
    fn test_preds() {
        let r = TestRequest::default().method(Method::TRACE).to_request();

        assert!(Not(Get()).check(&r,));
        assert!(!Not(Trace()).check(&r,));

        assert!(All(Trace()).and(Trace()).check(&r,));
        assert!(!All(Get()).and(Trace()).check(&r,));

        assert!(Any(Get()).or(Trace()).check(&r,));
        assert!(!Any(Get()).or(Get()).check(&r,));
    }
}
