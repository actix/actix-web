//! Lower-level types and re-exports.
//!
//! Most users will not have to interact with the types in this module, but it is useful for those
//! writing extractors, middleware, libraries, or interacting with the service API directly.

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

use crate::{http::header::ContentEncoding, HttpMessage as _};

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

/// Helper trait for managing response encoding.
///
/// Use `pre_encoded_with` to flag response as already encoded. For example, when serving a Gzip
/// compressed file from disk.
pub trait BodyEncoding {
    /// Get content encoding
    fn preferred_encoding(&self) -> Option<ContentEncoding>;

    /// Set content encoding to use.
    ///
    /// Must be used with [`Compress`] to take effect.
    ///
    /// [`Compress`]: crate::middleware::Compress
    fn encode_with(&mut self, encoding: ContentEncoding) -> &mut Self;

    // /// Flags that a file already is encoded so that [`Compress`] does not modify it.
    // ///
    // /// Effectively a shortcut for `compress_with("identity")`
    // /// plus `insert_header(ContentEncoding, encoding)`.
    // ///
    // /// [`Compress`]: crate::middleware::Compress
    // fn pre_encoded_with(&mut self, encoding: ContentEncoding) -> &mut Self;
}

struct CompressWith(ContentEncoding);

// TODO: add or delete this
// struct PreCompressed(ContentEncoding);

impl BodyEncoding for crate::HttpResponseBuilder {
    fn preferred_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<CompressWith>().map(|enc| enc.0)
    }

    fn encode_with(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(CompressWith(encoding));
        self
    }

    // fn pre_encoded_with(&mut self, encoding: ContentEncoding) -> &mut Self {
    //     self.extensions_mut().insert(PreCompressed(encoding));
    //     self
    // }
}

impl<B> BodyEncoding for crate::HttpResponse<B> {
    fn preferred_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<CompressWith>().map(|enc| enc.0)
    }

    fn encode_with(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(CompressWith(encoding));
        self
    }

    // fn pre_encoded_with(&mut self, encoding: ContentEncoding) -> &mut Self {
    //     self.extensions_mut().insert(PreCompressed(encoding));
    //     self
    // }
}

impl<B> BodyEncoding for ServiceResponse<B> {
    fn preferred_encoding(&self) -> Option<ContentEncoding> {
        self.request()
            .extensions()
            .get::<CompressWith>()
            .map(|enc| enc.0)
    }

    fn encode_with(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.request()
            .extensions_mut()
            .insert(CompressWith(encoding));
        self
    }

    // fn pre_encoded_with(&mut self, encoding: ContentEncoding) -> &mut Self {
    //     self.request()
    //         .extensions_mut()
    //         .insert(PreCompressed(encoding));
    //     self
    // }
}

// TODO: remove these impls ?
impl BodyEncoding for actix_http::ResponseBuilder {
    fn preferred_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<CompressWith>().map(|enc| enc.0)
    }

    fn encode_with(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(CompressWith(encoding));
        self
    }
}

impl<B> BodyEncoding for actix_http::Response<B> {
    fn preferred_encoding(&self) -> Option<ContentEncoding> {
        self.extensions().get::<CompressWith>().map(|enc| enc.0)
    }

    fn encode_with(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(CompressWith(encoding));
        self
    }
}
