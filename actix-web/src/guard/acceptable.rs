use super::{Guard, GuardContext};
use crate::http::header::{Accept, Quality};

/// A guard that verifies that an `Accept` header is present and it contains a compatible MIME type.
///
/// An exception is that matching `*/*` must be explicitly enabled because most browsers send this
/// as part of their `Accept` header for almost every request.
///
/// The most specific media range that matches the target type decides the outcome. The guard fails
/// when that range has a quality value of 0, so `Acceptable::new(mime::APPLICATION_JSON)` rejects
/// `Accept: application/*, application/json;q=0`.
///
/// # Examples
/// ```
/// use actix_web::{guard::Acceptable, web, HttpResponse};
///
/// web::resource("/images")
///     .guard(Acceptable::new(mime::IMAGE_STAR))
///     .default_service(web::to(|| async {
///         HttpResponse::Ok().body("only called when images responses are acceptable")
///     }));
/// ```
#[derive(Debug, Clone)]
pub struct Acceptable {
    mime: mime::Mime,

    /// Whether to match `*/*` mime type.
    ///
    /// Defaults to false because it's not very useful otherwise.
    match_star_star: bool,

    /// Whether to match structured syntax suffixes, such as `+json`.
    ///
    /// Defaults to false because the handler must be able to serve the concrete media type that
    /// matched, rather than only the configured MIME type.
    match_structured_suffix: bool,
}

impl Acceptable {
    /// Constructs new `Acceptable` guard with the given `mime` type/pattern.
    pub fn new(mime: mime::Mime) -> Self {
        Self {
            mime,
            match_star_star: false,
            match_structured_suffix: false,
        }
    }

    /// Allows `*/*` in the `Accept` header to pass the guard check.
    pub fn match_star_star(mut self) -> Self {
        self.match_star_star = true;
        self
    }

    /// Allows structured syntax suffixes of the configured subtype to pass the guard check.
    ///
    /// For example, `Acceptable::new(mime::APPLICATION_JSON).match_structured_suffix(true)` also
    /// matches `application/problem+json`. The handler must be able to serve the concrete media
    /// type that matched.
    pub fn match_structured_suffix(mut self, enabled: bool) -> Self {
        self.match_structured_suffix = enabled;
        self
    }
}

impl Guard for Acceptable {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        let accept = match ctx.header::<Accept>() {
            Some(hdr) => hdr,
            None => return false,
        };

        let target_type = self.mime.type_();
        let target_subtype = self.mime.subtype();
        let target_suffix = format!("+{target_subtype}");

        // The most specific matching media range decides the outcome, so that, for example,
        // `application/json;q=0` takes precedence over `application/*` (RFC 9110 §12.5.1).
        let mut best_match = None;

        for item in &accept.0 {
            let specificity = match (item.item.type_(), item.item.subtype()) {
                (typ, subtype)
                    if self.match_structured_suffix
                        && typ == target_type
                        && subtype
                            .strip_suffix(&target_suffix)
                            .is_some_and(|prefix| !prefix.is_empty()) =>
                {
                    2
                }
                (typ, subtype) if typ == target_type && subtype == target_subtype => 2,
                (typ, mime::STAR) if typ == target_type => 1,
                (mime::STAR, mime::STAR) if self.match_star_star => 0,
                _ => continue,
            };

            // media type parameters are not compared, so among equally specific ranges the
            // highest quality value is used
            best_match = best_match.max(Some((specificity, item.quality)));
        }

        // a quality value of 0 means "not acceptable" (RFC 9110 §12.4.2)
        matches!(best_match, Some((_, quality)) if quality > Quality::ZERO)
    }

    #[cfg(feature = "experimental-introspection")]
    fn name(&self) -> String {
        let mut options = String::new();
        if self.match_star_star {
            options.push_str(", match_star_star=true");
        }
        if self.match_structured_suffix {
            options.push_str(", match_structured_suffix=true");
        }
        format!("Acceptable({}{})", self.mime, options)
    }

    #[cfg(feature = "experimental-introspection")]
    fn details(&self) -> Option<Vec<super::GuardDetail>> {
        let mut details = Vec::new();
        details.push(super::GuardDetail::Generic(format!("mime={}", self.mime)));
        if self.match_star_star {
            details.push(super::GuardDetail::Generic(
                "match_star_star=true".to_string(),
            ));
        }
        if self.match_structured_suffix {
            details.push(super::GuardDetail::Generic(
                "match_structured_suffix=true".to_string(),
            ));
        }
        Some(details)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{http::header, test::TestRequest};

    #[test]
    fn test_acceptable() {
        let req = TestRequest::default().to_srv_request();
        assert!(!Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "application/json"))
            .to_srv_request();
        assert!(Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "text/html, application/json"))
            .to_srv_request();
        assert!(Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));
    }

    #[test]
    fn test_acceptable_zero_quality() {
        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "application/json;q=0"))
            .to_srv_request();
        assert!(!Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "application/*;q=0"))
            .to_srv_request();
        assert!(!Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "*/*;q=0"))
            .to_srv_request();
        assert!(!Acceptable::new(mime::APPLICATION_JSON)
            .match_star_star()
            .check(&req.guard_ctx()));

        // more specific range takes precedence, regardless of order
        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "application/*, application/json;q=0"))
            .to_srv_request();
        assert!(!Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "*/*, application/json;q=0"))
            .to_srv_request();
        assert!(!Acceptable::new(mime::APPLICATION_JSON)
            .match_star_star()
            .check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "application/*;q=0, application/json;q=0.5"))
            .to_srv_request();
        assert!(Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "*/*;q=0, application/*"))
            .to_srv_request();
        assert!(Acceptable::new(mime::APPLICATION_JSON)
            .match_star_star()
            .check(&req.guard_ctx()));

        // parameters are not compared, so an equally specific non-zero range still passes
        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "text/html;level=1;q=0, text/html"))
            .to_srv_request();
        assert!(Acceptable::new(mime::TEXT_HTML).check(&req.guard_ctx()));
    }

    #[test]
    fn test_acceptable_star() {
        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "text/html, */*;q=0.8"))
            .to_srv_request();

        assert!(Acceptable::new(mime::APPLICATION_JSON)
            .match_star_star()
            .check(&req.guard_ctx()));
    }

    #[test]
    fn test_acceptable_structured_suffix() {
        let guard = || Acceptable::new(mime::APPLICATION_JSON).match_structured_suffix(true);

        for accept in ["application/problem+json", "application/vnd.example+json"] {
            let req = TestRequest::default()
                .insert_header((header::ACCEPT, accept))
                .to_srv_request();
            assert!(guard().check(&req.guard_ctx()), "{accept}");
        }

        for accept in ["application/problem+json", "application/vnd.example+json"] {
            let req = TestRequest::default()
                .insert_header((header::ACCEPT, accept))
                .to_srv_request();
            assert!(!Acceptable::new(mime::APPLICATION_JSON).check(&req.guard_ctx()));
            assert!(!Acceptable::new(mime::APPLICATION_JSON)
                .match_structured_suffix(false)
                .check(&req.guard_ctx()));
        }

        for accept in ["text/example+json", "application/atom+xml"] {
            let req = TestRequest::default()
                .insert_header((header::ACCEPT, accept))
                .to_srv_request();
            assert!(!guard().check(&req.guard_ctx()), "{accept}");
        }

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "application/problem+json;q=0"))
            .to_srv_request();
        assert!(!guard().check(&req.guard_ctx()));

        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "*/*, application/json;q=0"))
            .to_srv_request();
        assert!(!guard().match_star_star().check(&req.guard_ctx()));
    }

    #[cfg(feature = "experimental-introspection")]
    #[test]
    fn acceptable_guard_details_include_mime() {
        let guard = Acceptable::new(mime::APPLICATION_JSON).match_star_star();
        let details = guard.details().expect("Missing guard details");

        assert!(details.iter().any(|detail| match detail {
            crate::guard::GuardDetail::Generic(value) => value == "match_star_star=true",
            _ => false,
        }));
        let expected = format!("mime={}", mime::APPLICATION_JSON);
        assert!(details.iter().any(|detail| match detail {
            crate::guard::GuardDetail::Generic(value) => value == &expected,
            _ => false,
        }));
        assert_eq!(
            guard.name(),
            format!(
                "Acceptable({}, match_star_star=true)",
                mime::APPLICATION_JSON
            )
        );
    }
}
