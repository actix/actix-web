use std::cell::{Ref, RefMut};
use std::rc::Rc;

use actix_http::body::{Body, MessageBody, ResponseBody};
use actix_http::http::{HeaderMap, Method, Uri, Version};
use actix_http::{
    Error, Extensions, HttpMessage, Payload, Request, RequestHead, Response,
    ResponseHead,
};
use actix_router::{Path, Url};
use futures::future::{ok, FutureResult, IntoFuture};

use crate::request::HttpRequest;

pub struct ServiceRequest<P> {
    req: HttpRequest,
    payload: Payload<P>,
}

impl<P> ServiceRequest<P> {
    pub(crate) fn new(
        path: Path<Url>,
        request: Request<P>,
        extensions: Rc<Extensions>,
    ) -> Self {
        let (head, payload) = request.into_parts();
        ServiceRequest {
            payload,
            req: HttpRequest::new(head, path, extensions),
        }
    }

    #[inline]
    pub fn into_request(self) -> HttpRequest {
        self.req
    }

    /// Create service response
    #[inline]
    pub fn into_response<B>(self, res: Response<B>) -> ServiceResponse<B> {
        ServiceResponse::new(self.req, res)
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<E: Into<Error>>(self, err: E) -> ServiceResponse {
        ServiceResponse::new(self.req, err.into().into())
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        &self.req.head
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head_mut(&mut self) -> &mut RequestHead {
        &mut self.req.head
    }

    /// Request's uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.head().uri
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.head().method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.head().uri.path()
    }

    #[inline]
    /// Returns Request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// The query string in the URL.
    ///
    /// E.g., id=10
    #[inline]
    pub fn query_string(&self) -> &str {
        if let Some(query) = self.uri().query().as_ref() {
            query
        } else {
            ""
        }
    }

    /// Get a reference to the Path parameters.
    ///
    /// Params is a container for url parameters.
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment.
    #[inline]
    pub fn match_info(&self) -> &Path<Url> {
        &self.req.path
    }

    #[inline]
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        &mut self.req.path
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.req.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.req.head.extensions_mut()
    }

    /// Application extensions
    #[inline]
    pub fn app_extensions(&self) -> &Extensions {
        self.req.app_extensions()
    }
}

impl<P> HttpMessage for ServiceRequest<P> {
    type Stream = P;

    #[inline]
    fn headers(&self) -> &HeaderMap {
        self.req.headers()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        std::mem::replace(&mut self.payload, Payload::None)
    }
}

impl<P> std::ops::Deref for ServiceRequest<P> {
    type Target = RequestHead;

    fn deref(&self) -> &RequestHead {
        self.req.head()
    }
}

impl<P> std::ops::DerefMut for ServiceRequest<P> {
    fn deref_mut(&mut self) -> &mut RequestHead {
        self.head_mut()
    }
}

pub struct ServiceFromRequest<P> {
    req: HttpRequest,
    payload: Payload<P>,
}

impl<P> ServiceFromRequest<P> {
    #[inline]
    pub fn into_request(self) -> HttpRequest {
        self.req
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<E: Into<Error>>(self, err: E) -> ServiceResponse {
        ServiceResponse::new(self.req, err.into().into())
    }
}

impl<P> std::ops::Deref for ServiceFromRequest<P> {
    type Target = HttpRequest;

    fn deref(&self) -> &HttpRequest {
        &self.req
    }
}

impl<P> HttpMessage for ServiceFromRequest<P> {
    type Stream = P;

    #[inline]
    fn headers(&self) -> &HeaderMap {
        self.req.headers()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        std::mem::replace(&mut self.payload, Payload::None)
    }
}

impl<P> From<ServiceRequest<P>> for ServiceFromRequest<P> {
    fn from(req: ServiceRequest<P>) -> Self {
        Self {
            req: req.req,
            payload: req.payload,
        }
    }
}

pub struct ServiceResponse<B = Body> {
    request: HttpRequest,
    response: Response<B>,
}

impl<B> ServiceResponse<B> {
    /// Create service response instance
    pub fn new(request: HttpRequest, response: Response<B>) -> Self {
        ServiceResponse { request, response }
    }

    /// Get reference to original request
    #[inline]
    pub fn request(&self) -> &HttpRequest {
        &self.request
    }

    /// Get reference to response
    #[inline]
    pub fn response(&self) -> &Response<B> {
        &self.response
    }

    /// Get mutable reference to response
    #[inline]
    pub fn response_mut(&mut self) -> &mut Response<B> {
        &mut self.response
    }

    /// Get the headers from the response
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        self.response.headers()
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.response.headers_mut()
    }
}

impl<B: MessageBody> ServiceResponse<B> {
    /// Set a new body
    pub fn map_body<F, B2: MessageBody>(self, f: F) -> ServiceResponse<B2>
    where
        F: FnOnce(&mut ResponseHead, ResponseBody<B>) -> ResponseBody<B2>,
    {
        let response = self.response.map_body(f);

        ServiceResponse {
            response,
            request: self.request,
        }
    }
}

impl<B: MessageBody> std::ops::Deref for ServiceResponse<B> {
    type Target = Response<B>;

    fn deref(&self) -> &Response<B> {
        self.response()
    }
}

impl<B: MessageBody> std::ops::DerefMut for ServiceResponse<B> {
    fn deref_mut(&mut self) -> &mut Response<B> {
        self.response_mut()
    }
}

impl<B: MessageBody> Into<Response<B>> for ServiceResponse<B> {
    fn into(self) -> Response<B> {
        self.response
    }
}

impl<B: MessageBody> IntoFuture for ServiceResponse<B> {
    type Item = ServiceResponse<B>;
    type Error = Error;
    type Future = FutureResult<ServiceResponse<B>, Error>;

    fn into_future(self) -> Self::Future {
        ok(self)
    }
}
