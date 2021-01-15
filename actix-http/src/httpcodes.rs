//! Status code based HTTP response builders.

#![allow(non_upper_case_globals)]

use http::StatusCode;

use crate::response::{Response, ResponseBuilder};

macro_rules! static_resp {
    ($name:ident, $status:expr) => {
        #[allow(missing_docs)]
        pub fn $name() -> ResponseBuilder {
            ResponseBuilder::new($status)
        }
    };
}

impl Response {
    static_resp!(kontinue, StatusCode::CONTINUE);
    static_resp!(switching_protocols, StatusCode::SWITCHING_PROTOCOLS);
    static_resp!(processing, StatusCode::PROCESSING);

    static_resp!(ok, StatusCode::OK);
    static_resp!(created, StatusCode::CREATED);
    static_resp!(accepted, StatusCode::ACCEPTED);
    static_resp!(
        non_authoritative_information,
        StatusCode::NON_AUTHORITATIVE_INFORMATION
    );

    static_resp!(no_content, StatusCode::NO_CONTENT);
    static_resp!(reset_content, StatusCode::RESET_CONTENT);
    static_resp!(partial_content, StatusCode::PARTIAL_CONTENT);
    static_resp!(multi_status, StatusCode::MULTI_STATUS);
    static_resp!(already_reported, StatusCode::ALREADY_REPORTED);

    static_resp!(multiple_choices, StatusCode::MULTIPLE_CHOICES);
    static_resp!(moved_permanently, StatusCode::MOVED_PERMANENTLY);
    static_resp!(found, StatusCode::FOUND);
    static_resp!(see_other, StatusCode::SEE_OTHER);
    static_resp!(not_modified, StatusCode::NOT_MODIFIED);
    static_resp!(use_proxy, StatusCode::USE_PROXY);
    static_resp!(temporary_redirect, StatusCode::TEMPORARY_REDIRECT);
    static_resp!(permanent_redirect, StatusCode::PERMANENT_REDIRECT);

    static_resp!(bad_request, StatusCode::BAD_REQUEST);
    static_resp!(not_found, StatusCode::NOT_FOUND);
    static_resp!(unauthorized, StatusCode::UNAUTHORIZED);
    static_resp!(payment_required, StatusCode::PAYMENT_REQUIRED);
    static_resp!(forbidden, StatusCode::FORBIDDEN);
    static_resp!(method_not_allowed, StatusCode::METHOD_NOT_ALLOWED);
    static_resp!(not_acceptable, StatusCode::NOT_ACCEPTABLE);
    static_resp!(
        proxy_authentication_required,
        StatusCode::PROXY_AUTHENTICATION_REQUIRED
    );
    static_resp!(request_timeout, StatusCode::REQUEST_TIMEOUT);
    static_resp!(conflict, StatusCode::CONFLICT);
    static_resp!(gone, StatusCode::GONE);
    static_resp!(length_required, StatusCode::LENGTH_REQUIRED);
    static_resp!(precondition_failed, StatusCode::PRECONDITION_FAILED);
    static_resp!(precondition_required, StatusCode::PRECONDITION_REQUIRED);
    static_resp!(payload_too_large, StatusCode::PAYLOAD_TOO_LARGE);
    static_resp!(uri_too_long, StatusCode::URI_TOO_LONG);
    static_resp!(unsupported_media_type, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    static_resp!(range_not_satisfiable, StatusCode::RANGE_NOT_SATISFIABLE);
    static_resp!(expectation_failed, StatusCode::EXPECTATION_FAILED);
    static_resp!(unprocessable_entity, StatusCode::UNPROCESSABLE_ENTITY);
    static_resp!(too_many_requests, StatusCode::TOO_MANY_REQUESTS);

    static_resp!(internal_server_error, StatusCode::INTERNAL_SERVER_ERROR);
    static_resp!(not_implemented, StatusCode::NOT_IMPLEMENTED);
    static_resp!(bad_gateway, StatusCode::BAD_GATEWAY);
    static_resp!(service_unavailable, StatusCode::SERVICE_UNAVAILABLE);
    static_resp!(gateway_timeout, StatusCode::GATEWAY_TIMEOUT);
    static_resp!(
        version_not_supported,
        StatusCode::HTTP_VERSION_NOT_SUPPORTED
    );
    static_resp!(variant_also_negotiates, StatusCode::VARIANT_ALSO_NEGOTIATES);
    static_resp!(insufficient_storage, StatusCode::INSUFFICIENT_STORAGE);
    static_resp!(loop_detected, StatusCode::LOOP_DETECTED);
}

#[cfg(test)]
mod tests {
    use crate::body::Body;
    use crate::response::Response;
    use http::StatusCode;

    #[test]
    fn test_build() {
        let resp = Response::ok().body(Body::Empty);
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
