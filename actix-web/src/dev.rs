//! Lower-level types and re-exports.
//!
//! Most users will not have to interact with the types in this module, but it is useful for those
//! writing extractors, middleware, libraries, or interacting with the service API directly.
//!
//! # Request Extractors
//! - [`ConnectionInfo`]: Connection information
//! - [`PeerAddr`]: Connection information

pub use actix_http::{Extensions, Payload, RequestHead, Response, ResponseHead};
pub use actix_router::{Path, ResourceDef, ResourcePath, Url};
pub use actix_server::{Server, ServerHandle};
pub use actix_service::{
    always_ready, fn_factory, fn_service, forward_ready, Service, ServiceFactory, Transform,
};

#[cfg(feature = "__compress")]
pub use actix_http::encoding::Decoder as Decompress;

pub use crate::config::{AppConfig, AppService};
#[doc(hidden)]
pub use crate::handler::Handler;
pub use crate::info::{ConnectionInfo, PeerAddr};
pub use crate::rmap::ResourceMap;
pub use crate::service::{HttpServiceFactory, ServiceRequest, ServiceResponse, WebService};

pub use crate::types::{JsonBody, Readlines, UrlEncoded};

use actix_router::Patterns;

pub(crate) fn ensure_leading_slash(mut patterns: Patterns) -> Patterns {
    match &mut patterns {
        Patterns::Single(pat) => {
            if !pat.is_empty() && !pat.starts_with('/') {
                pat.insert(0, '/');
            };
        }
        Patterns::List(pats) => {
            for pat in pats {
                if !pat.is_empty() && !pat.starts_with('/') {
                    pat.insert(0, '/');
                };
            }
        }
    }

    patterns
}
