use std::ops::{Deref, DerefMut};

use serde_urlencoded;
use serde::de::{self, DeserializeOwned};
use futures::future::{Future, FutureResult, result};

use body::Binary;
use error::Error;
use handler::FromRequest;
use httprequest::HttpRequest;
use httpmessage::{MessageBody, UrlEncoded};
use de::PathDeserializer;

/// Extract typed information from the request's path.
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Path, Result, http};
///
/// /// extract path info from "/{username}/{count}/?index.html" url
/// /// {username} - deserializes to a String
/// /// {count} -  - deserializes to a u32
/// fn index(info: Path<(String, u32)>) -> Result<String> {
///     Ok(format!("Welcome {}! {}", info.0, info.1))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/{username}/{count}/?index.html",       // <- define path parameters
///        |r| r.method(http::Method::GET).with(index));  // <- use `with` extractor
/// }
/// ```
///
/// It is possible to extract path information to a specific type that implements
/// `Deserialize` trait from *serde*.
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Path, Result, http};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract path info using serde
/// fn index(info: Path<Info>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/{username}/index.html",                // <- define path parameters
///        |r| r.method(http::Method::GET).with(index));  // <- use `with` extractor
/// }
/// ```
pub struct Path<T>{
    inner: T
}

impl<T> AsRef<T> for Path<T> {

    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<T> Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for Path<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Path<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T, S> FromRequest<S> for Path<T>
    where T: DeserializeOwned, S: 'static
{
    type Result = FutureResult<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>) -> Self::Result {
        let req = req.clone();
        result(de::Deserialize::deserialize(PathDeserializer::new(&req))
               .map_err(|e| e.into())
               .map(|inner| Path{inner}))
    }
}

/// Extract typed information from from the request's query.
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Query, http};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// // use `with` extractor for query info
/// // this handler get called only if request's query contains `username` field
/// fn index(info: Query<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html",
///        |r| r.method(http::Method::GET).with(index)); // <- use `with` extractor
/// }
/// ```
pub struct Query<T>(T);

impl<T> Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Query<T> {
    /// Deconstruct to a inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T, S> FromRequest<S> for Query<T>
    where T: de::DeserializeOwned, S: 'static
{
    type Result = FutureResult<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>) -> Self::Result {
        let req = req.clone();
        result(serde_urlencoded::from_str::<T>(req.query_string())
               .map_err(|e| e.into())
               .map(Query))
    }
}

/// Request payload extractor.
///
/// Loads request's payload and construct Binary instance.
impl<S: 'static> FromRequest<S> for Binary
{
    type Result = Box<Future<Item=Self, Error=Error>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>) -> Self::Result {
        Box::new(
            MessageBody::new(req.clone()).from_err().map(|b| b.into()))
    }
}

/// Extract typed information from the request's body.
///
/// To extract typed information from request's body, the type `T` must implement the
/// `Deserialize` trait from *serde*.
///
/// ## Example
///
/// It is possible to extract path information to a specific type that implements
/// `Deserialize` trait from *serde*.
///
/// ```rust
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Form, Result};
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// extract form data using serde
/// /// this handle get called only if content type is *x-www-form-urlencoded*
/// /// and content of the request could be deserialized to a `FormData` struct
/// fn index(form: Form<FormData>) -> Result<String> {
///     Ok(format!("Welcome {}!", form.username))
/// }
/// # fn main() {}
/// ```
pub struct Form<T>(pub T);

impl<T> Deref for Form<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Form<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, S> FromRequest<S> for Form<T>
    where T: DeserializeOwned + 'static, S: 'static
{
    type Result = Box<Future<Item=Self, Error=Error>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>) -> Self::Result {
        Box::new(UrlEncoded::new(req.clone()).from_err().map(Form))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::{Async, Future};
    use http::header;
    use router::{Router, Resource};
    use resource::ResourceHandler;
    use test::TestRequest;
    use server::ServerSettings;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[test]
    fn test_binary() {
        let mut req = TestRequest::with_header(header::CONTENT_LENGTH, "11").finish();
        req.payload_mut().unread_data(Bytes::from_static(b"hello=world"));

        match Binary::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s, Binary::from(Bytes::from_static(b"hello=world")));
            },
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_form() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::CONTENT_LENGTH, "11")
            .finish();
        req.payload_mut().unread_data(Bytes::from_static(b"hello=world"));

        match Form::<Info>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.hello, "world");
            },
            _ => unreachable!(),
        }
    }


    #[derive(Deserialize)]
    struct MyStruct {
        key: String,
        value: String,
    }

    #[derive(Deserialize)]
    struct Id {
        id: String,
    }

    #[derive(Deserialize)]
    struct Test2 {
        key: String,
        value: u32,
    }

    #[test]
    fn test_request_extract() {
        let mut req = TestRequest::with_uri("/name/user1/?id=test").finish();

        let mut resource = ResourceHandler::<()>::default();
        resource.name("index");
        let mut routes = Vec::new();
        routes.push((Resource::new("index", "/{key}/{value}/"), Some(resource)));
        let (router, _) = Router::new("", ServerSettings::default(), routes);
        assert!(router.recognize(&mut req).is_some());

        match Path::<MyStruct>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.key, "name");
                assert_eq!(s.value, "user1");
            },
            _ => unreachable!(),
        }

        match Path::<(String, String)>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.0, "name");
                assert_eq!(s.1, "user1");
            },
            _ => unreachable!(),
        }

        match Query::<Id>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.id, "test");
            },
            _ => unreachable!(),
        }

        let mut req = TestRequest::with_uri("/name/32/").finish();
        assert!(router.recognize(&mut req).is_some());

        match Path::<Test2>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.as_ref().key, "name");
                assert_eq!(s.value, 32);
            },
            _ => unreachable!(),
        }

        match Path::<(String, u8)>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.0, "name");
                assert_eq!(s.1, 32);
            },
            _ => unreachable!(),
        }

        match Path::<Vec<String>>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.into_inner(), vec!["name".to_owned(), "32".to_owned()]);
            },
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_extract_path_signle() {
        let mut resource = ResourceHandler::<()>::default();
        resource.name("index");
        let mut routes = Vec::new();
        routes.push((Resource::new("index", "/{value}/"), Some(resource)));
        let (router, _) = Router::new("", ServerSettings::default(), routes);

        let mut req = TestRequest::with_uri("/32/").finish();
        assert!(router.recognize(&mut req).is_some());

        match Path::<i8>::from_request(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.into_inner(), 32);
            },
            _ => unreachable!(),
        }
    }
}
