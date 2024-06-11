//! Common header names not defined in [`http`].
//!
//! Any headers added to this file will need to be re-exported from the list at `crate::headers`.

use http::header::HeaderName;

/// Response header field that indicates how caches have handled that response and its corresponding
/// request.
///
/// See [RFC 9211](https://www.rfc-editor.org/rfc/rfc9211) for full semantics.
// TODO(breaking): replace with http's version
pub const CACHE_STATUS: HeaderName = HeaderName::from_static("cache-status");

/// Response header field that allows origin servers to control the behavior of CDN caches
/// interposed between them and clients separately from other caches that might handle the response.
///
/// See [RFC 9213](https://www.rfc-editor.org/rfc/rfc9213) for full semantics.
// TODO(breaking): replace with http's version
pub const CDN_CACHE_CONTROL: HeaderName = HeaderName::from_static("cdn-cache-control");

/// Response header that prevents a document from loading any cross-origin resources that don't
/// explicitly grant the document permission (using [CORP] or [CORS]).
///
/// [CORP]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Cross-Origin_Resource_Policy_(CORP)
/// [CORS]: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS
pub const CROSS_ORIGIN_EMBEDDER_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-embedder-policy");

/// Response header that allows you to ensure a top-level document does not share a browsing context
/// group with cross-origin documents.
pub const CROSS_ORIGIN_OPENER_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-opener-policy");

/// Response header that conveys a desire that the browser blocks no-cors cross-origin/cross-site
/// requests to the given resource.
pub const CROSS_ORIGIN_RESOURCE_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-resource-policy");

/// Response header that provides a mechanism to allow and deny the use of browser features in a
/// document or within any `<iframe>` elements in the document.
pub const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");

/// Request header (de-facto standard) for identifying the originating IP address of a client
/// connecting to a web server through a proxy server.
pub const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// Request header (de-facto standard) for identifying the original host requested by the client in
/// the `Host` HTTP request header.
pub const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");

/// Request header (de-facto standard) for identifying the protocol that a client used to connect to
/// your proxy or load balancer.
pub const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
