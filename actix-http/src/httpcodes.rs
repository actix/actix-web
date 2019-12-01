//! Basic http responses
#![allow(non_upper_case_globals)]
use http::StatusCode;

use crate::response::{Response, ResponseBuilder};

macro_rules! STATIC_RESP {
    ($name:ident, $status:expr) => {
        #[allow(non_snake_case, missing_docs)]
        pub fn $name() -> ResponseBuilder {
            ResponseBuilder::new($status)
        }
    };
}

impl Response {
    STATIC_RESP!(Ok, StatusCode::OK);
    STATIC_RESP!(Created, StatusCode::CREATED);
    STATIC_RESP!(Accepted, StatusCode::ACCEPTED);
    STATIC_RESP!(
        NonAuthoritativeInformation,
        StatusCode::NON_AUTHORITATIVE_INFORMATION
    );

    STATIC_RESP!(NoContent, StatusCode::NO_CONTENT);
    STATIC_RESP!(ResetContent, StatusCode::RESET_CONTENT);
    STATIC_RESP!(PartialContent, StatusCode::PARTIAL_CONTENT);
    STATIC_RESP!(MultiStatus, StatusCode::MULTI_STATUS);
    STATIC_RESP!(AlreadyReported, StatusCode::ALREADY_REPORTED);

    STATIC_RESP!(MultipleChoices, StatusCode::MULTIPLE_CHOICES);
    STATIC_RESP!(MovedPermanently, StatusCode::MOVED_PERMANENTLY);
    STATIC_RESP!(Found, StatusCode::FOUND);
    STATIC_RESP!(SeeOther, StatusCode::SEE_OTHER);
    STATIC_RESP!(NotModified, StatusCode::NOT_MODIFIED);
    STATIC_RESP!(UseProxy, StatusCode::USE_PROXY);
    STATIC_RESP!(TemporaryRedirect, StatusCode::TEMPORARY_REDIRECT);
    STATIC_RESP!(PermanentRedirect, StatusCode::PERMANENT_REDIRECT);

    STATIC_RESP!(BadRequest, StatusCode::BAD_REQUEST);
    STATIC_RESP!(NotFound, StatusCode::NOT_FOUND);
    STATIC_RESP!(Unauthorized, StatusCode::UNAUTHORIZED);
    STATIC_RESP!(PaymentRequired, StatusCode::PAYMENT_REQUIRED);
    STATIC_RESP!(Forbidden, StatusCode::FORBIDDEN);
    STATIC_RESP!(MethodNotAllowed, StatusCode::METHOD_NOT_ALLOWED);
    STATIC_RESP!(NotAcceptable, StatusCode::NOT_ACCEPTABLE);
    STATIC_RESP!(
        ProxyAuthenticationRequired,
        StatusCode::PROXY_AUTHENTICATION_REQUIRED
    );
    STATIC_RESP!(RequestTimeout, StatusCode::REQUEST_TIMEOUT);
    STATIC_RESP!(Conflict, StatusCode::CONFLICT);
    STATIC_RESP!(Gone, StatusCode::GONE);
    STATIC_RESP!(LengthRequired, StatusCode::LENGTH_REQUIRED);
    STATIC_RESP!(PreconditionFailed, StatusCode::PRECONDITION_FAILED);
    STATIC_RESP!(PreconditionRequired, StatusCode::PRECONDITION_REQUIRED);
    STATIC_RESP!(PayloadTooLarge, StatusCode::PAYLOAD_TOO_LARGE);
    STATIC_RESP!(UriTooLong, StatusCode::URI_TOO_LONG);
    STATIC_RESP!(UnsupportedMediaType, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    STATIC_RESP!(RangeNotSatisfiable, StatusCode::RANGE_NOT_SATISFIABLE);
    STATIC_RESP!(ExpectationFailed, StatusCode::EXPECTATION_FAILED);
    STATIC_RESP!(UnprocessableEntity, StatusCode::UNPROCESSABLE_ENTITY);
    STATIC_RESP!(TooManyRequests, StatusCode::TOO_MANY_REQUESTS);

    STATIC_RESP!(InternalServerError, StatusCode::INTERNAL_SERVER_ERROR);
    STATIC_RESP!(NotImplemented, StatusCode::NOT_IMPLEMENTED);
    STATIC_RESP!(BadGateway, StatusCode::BAD_GATEWAY);
    STATIC_RESP!(ServiceUnavailable, StatusCode::SERVICE_UNAVAILABLE);
    STATIC_RESP!(GatewayTimeout, StatusCode::GATEWAY_TIMEOUT);
    STATIC_RESP!(VersionNotSupported, StatusCode::HTTP_VERSION_NOT_SUPPORTED);
    STATIC_RESP!(VariantAlsoNegotiates, StatusCode::VARIANT_ALSO_NEGOTIATES);
    STATIC_RESP!(InsufficientStorage, StatusCode::INSUFFICIENT_STORAGE);
    STATIC_RESP!(LoopDetected, StatusCode::LOOP_DETECTED);
}

#[cfg(test)]
mod tests {
    use crate::body::Body;
    use crate::response::Response;
    use http::StatusCode;

    #[test]
    fn test_build() {
        let resp = Response::Ok().body(Body::Empty);
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
