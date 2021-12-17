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

pub use crate::types::form::UrlEncoded;
pub use crate::types::json::JsonBody;
pub use crate::types::readlines::Readlines;

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

// TODO: remove this if it doesn't appear to be needed

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum AnyBody {
    None,
    Full { body: crate::web::Bytes },
    Boxed { body: actix_http::body::BoxBody },
}

impl crate::body::MessageBody for AnyBody {
    type Error = crate::BoxError;

    /// Body size hint.
    fn size(&self) -> crate::body::BodySize {
        match self {
            AnyBody::None => crate::body::BodySize::None,
            AnyBody::Full { body } => body.size(),
            AnyBody::Boxed { body } => body.size(),
        }
    }

    /// Attempt to pull out the next chunk of body bytes.
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<crate::web::Bytes, Self::Error>>> {
        match self.get_mut() {
            AnyBody::None => std::task::Poll::Ready(None),
            AnyBody::Full { body } => {
                let bytes = std::mem::take(body);
                std::task::Poll::Ready(Some(Ok(bytes)))
            }
            AnyBody::Boxed { body } => body.as_pin_mut().poll_next(cx),
        }
    }

    fn try_into_bytes(self) -> Result<crate::web::Bytes, Self> {
        match self {
            AnyBody::None => Ok(crate::web::Bytes::new()),
            AnyBody::Full { body } => Ok(body),
            _ => Err(self),
        }
    }
}
