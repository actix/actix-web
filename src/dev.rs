//! Lower-level types and re-exports.
//!
//! Most users will not have to interact with the types in this module, but it is useful for those
//! writing extractors, middleware, libraries, or interacting with the service API directly.

pub use crate::config::{AppConfig, AppService};
#[doc(hidden)]
pub use crate::handler::Handler;
pub use crate::info::{ConnectionInfo, PeerAddr};
pub use crate::rmap::ResourceMap;
pub use crate::service::{HttpServiceFactory, ServiceRequest, ServiceResponse, WebService};

pub use crate::types::{JsonBody, Readlines, UrlEncoded};

pub use actix_http::{Extensions, Payload, PayloadStream, RequestHead, Response, ResponseHead};
pub use actix_router::{Path, ResourceDef, ResourcePath, Url};
pub use actix_server::{Server, ServerHandle};
pub use actix_service::{
    always_ready, fn_factory, fn_service, forward_ready, Service, ServiceFactory, Transform,
};

#[cfg(feature = "__compress")]
pub use actix_http::encoding::Decoder as Decompress;

use crate::http::header::ContentEncoding;

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
struct Enc(ContentEncoding);

/// Helper trait that allows to set specific encoding for response.
pub trait BodyEncoding {
    /// Get content encoding
    fn get_encoding(&self) -> Option<ContentEncoding>;

    /// Set content encoding
    ///
    /// Must be used with [`crate::middleware::Compress`] to take effect.
    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self;
}

impl BodyEncoding for actix_http::ResponseBuilder {
    fn get_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<Enc>().map(|enc| enc.0)
    }

    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(Enc(encoding));
        self
    }
}

impl<B> BodyEncoding for actix_http::Response<B> {
    fn get_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<Enc>().map(|enc| enc.0)
    }

    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(Enc(encoding));
        self
    }
}

impl BodyEncoding for crate::HttpResponseBuilder {
    fn get_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<Enc>().map(|enc| enc.0)
    }

    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(Enc(encoding));
        self
    }
}

impl<B> BodyEncoding for crate::HttpResponse<B> {
    fn get_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<Enc>().map(|enc| enc.0)
    }

    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(Enc(encoding));
        self
    }
}
