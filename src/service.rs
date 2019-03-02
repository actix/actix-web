use std::rc::Rc;

use actix_http::body::{Body, MessageBody, ResponseBody};
use actix_http::http::HeaderMap;
use actix_http::{
    Error, Extensions, HttpMessage, Payload, Request, Response, ResponseHead,
};
use actix_router::{Path, Url};

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
    pub fn request(&self) -> &HttpRequest {
        &self.req
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

    #[inline]
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        &mut self.req.path
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
    type Target = HttpRequest;

    fn deref(&self) -> &HttpRequest {
        self.request()
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
