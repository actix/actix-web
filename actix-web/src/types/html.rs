//! Semantic HTML responder. See [`Html`].

use crate::{
    http::{
        header::{self, ContentType, TryIntoHeaderValue},
        StatusCode,
    },
    HttpRequest, HttpResponse, Responder,
};

/// Semantic HTML responder.
///
/// When used as a responder, creates a 200 OK response, sets the correct HTML content type, and
/// uses the string passed to [`Html::new()`] as the body.
///
/// ```
/// # use actix_web::web::Html;
/// Html::new("<p>Hello, World!</p>")
/// # ;
/// ```
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct Html(String);

impl Html {
    /// Constructs a new `Html` responder.
    pub fn new(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}

impl Responder for Html {
    type Body = String;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        let mut res = HttpResponse::with_body(StatusCode::OK, self.0);
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            ContentType::html().try_into_value().unwrap(),
        );
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::TestRequest;

    #[test]
    fn responder() {
        let req = TestRequest::default().to_http_request();

        let res = Html::new("<p>Hello, World!</p>");
        let res = res.respond_to(&req);

        assert!(res.status().is_success());
        assert!(res
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert!(res.body().starts_with("<p>"));
    }
}
