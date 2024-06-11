use super::{Guard, GuardContext};
use crate::http::header::Accept;

/// A guard that verifies that an `Accept` header is present and it contains a compatible MIME type.
///
/// An exception is that matching `*/*` must be explicitly enabled because most browsers send this
/// as part of their `Accept` header for almost every request.
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
}

impl Acceptable {
    /// Constructs new `Acceptable` guard with the given `mime` type/pattern.
    pub fn new(mime: mime::Mime) -> Self {
        Self {
            mime,
            match_star_star: false,
        }
    }

    /// Allows `*/*` in the `Accept` header to pass the guard check.
    pub fn match_star_star(mut self) -> Self {
        self.match_star_star = true;
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

        for mime in accept.0.into_iter().map(|q| q.item) {
            return match (mime.type_(), mime.subtype()) {
                (typ, subtype) if typ == target_type && subtype == target_subtype => true,
                (typ, mime::STAR) if typ == target_type => true,
                (mime::STAR, mime::STAR) if self.match_star_star => true,
                _ => continue,
            };
        }

        false
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
    fn test_acceptable_star() {
        let req = TestRequest::default()
            .insert_header((header::ACCEPT, "text/html, */*;q=0.8"))
            .to_srv_request();

        assert!(Acceptable::new(mime::APPLICATION_JSON)
            .match_star_star()
            .check(&req.guard_ctx()));
    }
}
