//! Status code based HTTP response builders.

use actix_http::StatusCode;

use crate::{HttpResponse, HttpResponseBuilder};

macro_rules! static_resp {
    ($name:ident, $status:expr) => {
        #[allow(non_snake_case, missing_docs)]
        pub fn $name() -> HttpResponseBuilder {
            HttpResponseBuilder::new($status)
        }
    };
}

impl HttpResponse {
    static_resp!(Continue, StatusCode::CONTINUE);
    static_resp!(SwitchingProtocols, StatusCode::SWITCHING_PROTOCOLS);
    static_resp!(Processing, StatusCode::PROCESSING);

    static_resp!(Ok, StatusCode::OK);
    static_resp!(Created, StatusCode::CREATED);
    static_resp!(Accepted, StatusCode::ACCEPTED);
    static_resp!(
        NonAuthoritativeInformation,
        StatusCode::NON_AUTHORITATIVE_INFORMATION
    );
    static_resp!(NoContent, StatusCode::NO_CONTENT);
    static_resp!(ResetContent, StatusCode::RESET_CONTENT);
    static_resp!(PartialContent, StatusCode::PARTIAL_CONTENT);
    static_resp!(MultiStatus, StatusCode::MULTI_STATUS);
    static_resp!(AlreadyReported, StatusCode::ALREADY_REPORTED);
    static_resp!(ImUsed, StatusCode::IM_USED);

    static_resp!(MultipleChoices, StatusCode::MULTIPLE_CHOICES);
    static_resp!(MovedPermanently, StatusCode::MOVED_PERMANENTLY);
    static_resp!(Found, StatusCode::FOUND);
    static_resp!(SeeOther, StatusCode::SEE_OTHER);
    static_resp!(NotModified, StatusCode::NOT_MODIFIED);
    static_resp!(UseProxy, StatusCode::USE_PROXY);
    static_resp!(TemporaryRedirect, StatusCode::TEMPORARY_REDIRECT);
    static_resp!(PermanentRedirect, StatusCode::PERMANENT_REDIRECT);

    static_resp!(BadRequest, StatusCode::BAD_REQUEST);
    static_resp!(Unauthorized, StatusCode::UNAUTHORIZED);
    static_resp!(PaymentRequired, StatusCode::PAYMENT_REQUIRED);
    static_resp!(Forbidden, StatusCode::FORBIDDEN);
    static_resp!(NotFound, StatusCode::NOT_FOUND);
    static_resp!(MethodNotAllowed, StatusCode::METHOD_NOT_ALLOWED);
    static_resp!(NotAcceptable, StatusCode::NOT_ACCEPTABLE);
    static_resp!(
        ProxyAuthenticationRequired,
        StatusCode::PROXY_AUTHENTICATION_REQUIRED
    );
    static_resp!(RequestTimeout, StatusCode::REQUEST_TIMEOUT);
    static_resp!(Conflict, StatusCode::CONFLICT);
    static_resp!(Gone, StatusCode::GONE);
    static_resp!(LengthRequired, StatusCode::LENGTH_REQUIRED);
    static_resp!(PreconditionFailed, StatusCode::PRECONDITION_FAILED);
    static_resp!(PayloadTooLarge, StatusCode::PAYLOAD_TOO_LARGE);
    static_resp!(UriTooLong, StatusCode::URI_TOO_LONG);
    static_resp!(UnsupportedMediaType, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    static_resp!(RangeNotSatisfiable, StatusCode::RANGE_NOT_SATISFIABLE);
    static_resp!(ExpectationFailed, StatusCode::EXPECTATION_FAILED);
    static_resp!(ImATeapot, StatusCode::IM_A_TEAPOT);
    static_resp!(MisdirectedRequest, StatusCode::MISDIRECTED_REQUEST);
    static_resp!(UnprocessableEntity, StatusCode::UNPROCESSABLE_ENTITY);
    static_resp!(Locked, StatusCode::LOCKED);
    static_resp!(FailedDependency, StatusCode::FAILED_DEPENDENCY);
    static_resp!(UpgradeRequired, StatusCode::UPGRADE_REQUIRED);
    static_resp!(PreconditionRequired, StatusCode::PRECONDITION_REQUIRED);
    static_resp!(TooManyRequests, StatusCode::TOO_MANY_REQUESTS);
    static_resp!(
        RequestHeaderFieldsTooLarge,
        StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
    );
    static_resp!(
        UnavailableForLegalReasons,
        StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS
    );

    static_resp!(InternalServerError, StatusCode::INTERNAL_SERVER_ERROR);
    static_resp!(NotImplemented, StatusCode::NOT_IMPLEMENTED);
    static_resp!(BadGateway, StatusCode::BAD_GATEWAY);
    static_resp!(ServiceUnavailable, StatusCode::SERVICE_UNAVAILABLE);
    static_resp!(GatewayTimeout, StatusCode::GATEWAY_TIMEOUT);
    static_resp!(VersionNotSupported, StatusCode::HTTP_VERSION_NOT_SUPPORTED);
    static_resp!(VariantAlsoNegotiates, StatusCode::VARIANT_ALSO_NEGOTIATES);
    static_resp!(InsufficientStorage, StatusCode::INSUFFICIENT_STORAGE);
    static_resp!(LoopDetected, StatusCode::LOOP_DETECTED);
    static_resp!(NotExtended, StatusCode::NOT_EXTENDED);
    static_resp!(
        NetworkAuthenticationRequired,
        StatusCode::NETWORK_AUTHENTICATION_REQUIRED
    );
}

#[cfg(test)]
mod tests {
    use crate::{http::StatusCode, HttpResponse};

    #[test]
    fn test_build() {
        let resp = HttpResponse::Ok().finish();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
