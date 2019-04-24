use std::cell::{Ref, RefMut};

use actix_codec::Framed;
use actix_http::http::{HeaderMap, Method, Uri, Version};
use actix_http::{h1::Codec, Extensions, Request, RequestHead};
use actix_router::{Path, Url};

use crate::state::State;

pub struct FramedRequest<Io, S = ()> {
    req: Request,
    framed: Framed<Io, Codec>,
    state: State<S>,
    pub(crate) path: Path<Url>,
}

impl<Io, S> FramedRequest<Io, S> {
    pub fn new(
        req: Request,
        framed: Framed<Io, Codec>,
        path: Path<Url>,
        state: State<S>,
    ) -> Self {
        Self {
            req,
            framed,
            state,
            path,
        }
    }
}

impl<Io, S> FramedRequest<Io, S> {
    /// Split request into a parts
    pub fn into_parts(self) -> (Request, Framed<Io, Codec>, State<S>) {
        (self.req, self.framed, self.state)
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        self.req.head()
    }

    /// This method returns muttable reference to the request head.
    /// panics if multiple references of http request exists.
    #[inline]
    pub fn head_mut(&mut self) -> &mut RequestHead {
        self.req.head_mut()
    }

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        self.state.get_ref()
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

    #[inline]
    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
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

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.head().extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.head().extensions_mut()
    }
}

#[cfg(test)]
mod tests {
    use actix_http::http::{HeaderName, HeaderValue, HttpTryFrom};
    use actix_http::test::{TestBuffer, TestRequest};

    use super::*;

    #[test]
    fn test_reqest() {
        let buf = TestBuffer::empty();
        let framed = Framed::new(buf, Codec::default());
        let req = TestRequest::with_uri("/index.html?q=1")
            .header("content-type", "test")
            .finish();
        let path = Path::new(Url::new(req.uri().clone()));

        let mut freq = FramedRequest::new(req, framed, path, State::new(10u8));
        assert_eq!(*freq.state(), 10);
        assert_eq!(freq.version(), Version::HTTP_11);
        assert_eq!(freq.method(), Method::GET);
        assert_eq!(freq.path(), "/index.html");
        assert_eq!(freq.query_string(), "q=1");
        assert_eq!(
            freq.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "test"
        );

        freq.head_mut().headers.insert(
            HeaderName::try_from("x-hdr").unwrap(),
            HeaderValue::from_static("test"),
        );
        assert_eq!(
            freq.headers().get("x-hdr").unwrap().to_str().unwrap(),
            "test"
        );

        freq.extensions_mut().insert(100usize);
        assert_eq!(*freq.extensions().get::<usize>().unwrap(), 100usize);

        let (_, _, state) = freq.into_parts();
        assert_eq!(*state, 10);
    }
}
