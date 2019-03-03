use std::cell::{Ref, RefMut};
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use actix_http::http::{HeaderMap, Method, Uri, Version};
use actix_http::{Error, Extensions, HttpMessage, Message, Payload, RequestHead};
use actix_router::{Path, Url};

use crate::extract::FromRequest;
use crate::service::ServiceFromRequest;

#[derive(Clone)]
/// An HTTP Request
pub struct HttpRequest {
    pub(crate) head: Message<RequestHead>,
    pub(crate) path: Path<Url>,
    extensions: Rc<Extensions>,
}

impl HttpRequest {
    #[inline]
    pub(crate) fn new(
        head: Message<RequestHead>,
        path: Path<Url>,
        extensions: Rc<Extensions>,
    ) -> HttpRequest {
        HttpRequest {
            head,
            path,
            extensions,
        }
    }
}

impl HttpRequest {
    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        &self.head
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
        &self.path
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.head.extensions_mut()
    }

    /// Application extensions
    #[inline]
    pub fn app_extensions(&self) -> &Extensions {
        &self.extensions
    }

    // /// Get *ConnectionInfo* for the correct request.
    // #[inline]
    // pub fn connection_info(&self) -> Ref<ConnectionInfo> {
    //    ConnectionInfo::get(&*self)
    // }
}

impl Deref for HttpRequest {
    type Target = RequestHead;

    fn deref(&self) -> &RequestHead {
        self.head()
    }
}

impl HttpMessage for HttpRequest {
    type Stream = ();

    #[inline]
    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        Payload::None
    }
}

/// It is possible to get `HttpRequest` as an extractor handler parameter
///
/// ## Example
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, HttpRequest};
///
/// /// extract `Thing` from request
/// fn index(req: HttpRequest) -> String {
///    format!("Got thing: {:?}", req)
/// }
///
/// fn main() {
///     let app = App::new().resource("/users/:first", |r| {
///         r.route(web::get().to(index))
///     });
/// }
/// ```
impl<P> FromRequest<P> for HttpRequest {
    type Error = Error;
    type Future = Result<Self, Error>;
    type Config = ();

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Ok(req.clone())
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "\nHttpRequest {:?} {}:{}",
            self.head.version,
            self.head.method,
            self.path()
        )?;
        if !self.query_string().is_empty() {
            writeln!(f, "  query: ?{:?}", self.query_string())?;
        }
        if !self.match_info().is_empty() {
            writeln!(f, "  params: {:?}", self.match_info())?;
        }
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}
