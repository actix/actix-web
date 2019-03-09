use std::cell::{Ref, RefMut};
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use actix_http::http::{HeaderMap, Method, Uri, Version};
use actix_http::{Error, Extensions, HttpMessage, Message, Payload, RequestHead};
use actix_router::{Path, Url};

use crate::error::UrlGenerationError;
use crate::extract::FromRequest;
use crate::rmap::ResourceMap;
use crate::service::ServiceFromRequest;

#[derive(Clone)]
/// An HTTP Request
pub struct HttpRequest {
    pub(crate) head: Message<RequestHead>,
    pub(crate) path: Path<Url>,
    rmap: Rc<ResourceMap>,
    extensions: Rc<Extensions>,
}

impl HttpRequest {
    #[inline]
    pub(crate) fn new(
        head: Message<RequestHead>,
        path: Path<Url>,
        rmap: Rc<ResourceMap>,
        extensions: Rc<Extensions>,
    ) -> HttpRequest {
        HttpRequest {
            head,
            path,
            rmap,
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

    /// Application extensions
    #[inline]
    pub fn app_extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Generate url for named resource
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::{App, HttpRequest, HttpResponse, http};
    /// #
    /// fn index(req: HttpRequest) -> HttpResponse {
    ///     let url = req.url_for("foo", &["1", "2", "3"]); // <- generate url for "foo" resource
    ///     HttpResponse::Ok().into()
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .resource("/test/{one}/{two}/{three}", |r| {
    ///              r.name("foo");  // <- set resource name, then it could be used in `url_for`
    ///              r.method(http::Method::GET).f(|_| HttpResponse::Ok());
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn url_for<U, I>(
        &self,
        name: &str,
        elements: U,
    ) -> Result<url::Url, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        self.rmap.url_for(&self, name, elements)
    }

    /// Generate url for named resource
    ///
    /// This method is similar to `HttpRequest::url_for()` but it can be used
    /// for urls that do not contain variable parts.
    pub fn url_for_static(&self, name: &str) -> Result<url::Url, UrlGenerationError> {
        const NO_PARAMS: [&str; 0] = [];
        self.url_for(name, &NO_PARAMS)
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
    /// Returns Request's headers.
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Request extensions
    #[inline]
    fn extensions(&self) -> Ref<Extensions> {
        self.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.head.extensions_mut()
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
///     let app = App::new().service(
///         web::resource("/users/{first}").route(
///             web::get().to(index))
///     );
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
