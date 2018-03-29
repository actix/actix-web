use std::ops::{Deref, DerefMut};

use serde_urlencoded;
use serde::de::{self, Deserializer, DeserializeOwned, Visitor, Error as DeError};
use futures::future::{Future, FutureResult, result};

use error::Error;
use httprequest::HttpRequest;


pub trait HttpRequestExtractor<S>: Sized where S: 'static
{
    type Result: Future<Item=Self, Error=Error>;

    fn extract(req: &HttpRequest<S>) -> Self::Result;
}

impl<S: 'static> HttpRequestExtractor<S> for HttpRequest<S>
{
    type Result = FutureResult<Self, Error>;

    #[inline]
    fn extract(req: &HttpRequest<S>) -> Self::Result {
        result(Ok(req.clone()))
    }
}

/// Extract typed information from the request's path.
///
/// `S` - application state type
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// use actix_web::Path;
///
/// /// Application state
/// struct State {}
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// extract path info using serde
/// fn index(info: Path<Info, State>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = Application::with_state(State{}).resource(
///        "/{username}/index.html",                // <- define path parameters
///        |r| r.method(Method::GET).with(index));  // <- use `with` extractor
/// }
/// ```
pub struct Path<T, S=()>{
    item: T,
    req: HttpRequest<S>,
}

impl<T, S> Deref for Path<T, S> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.item
    }
}

impl<T, S> DerefMut for Path<T, S> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.item
    }
}

impl<T, S> Path<T, S> {

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        self.req.state()
    }

    /// Incoming request
    #[inline]
    pub fn request(&self) -> &HttpRequest<S> {
        &self.req
    }

    /// Deconstruct instance into parts
    pub fn into(self) -> (T, HttpRequest<S>) {
        (self.item, self.req)
    }

}

impl<T, S> HttpRequestExtractor<S> for Path<T, S>
    where T: DeserializeOwned, S: 'static
{
    type Result = FutureResult<Self, Error>;

    #[inline]
    fn extract(req: &HttpRequest<S>) -> Self::Result {
        let req = req.clone();
        result(de::Deserialize::deserialize(PathExtractor{req: &req})
               .map_err(|e| e.into())
               .map(|item| Path{item, req}))
    }
}

/// Extract typed information from from the request's query.
///
/// `S` - application state type
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// use actix_web::Query;
///
/// /// Application state
/// struct State {}
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// // use `with` extractor for query info
/// // this handler get called only if request's query contains `username` field
/// fn index(info: Query<Info, State>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = Application::with_state(State{}).resource(
///        "/index.html",
///        |r| r.method(Method::GET).with(index)); // <- use `with` extractor
/// }
/// ```
pub struct Query<T, S=()>{
    item: T,
    req: HttpRequest<S>,
}

impl<T, S> Deref for Query<T, S> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.item
    }
}

impl<T, S> DerefMut for Query<T, S> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.item
    }
}

impl<T, S> Query<T, S> {

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        self.req.state()
    }

    /// Incoming request
    #[inline]
    pub fn request(&self) -> &HttpRequest<S> {
        &self.req
    }

    /// Deconstruct instance into parts
    pub fn into(self) -> (T, HttpRequest<S>) {
        (self.item, self.req)
    }
}

impl<T, S> HttpRequestExtractor<S> for Query<T, S>
    where T: de::DeserializeOwned, S: 'static
{
    type Result = FutureResult<Self, Error>;

    #[inline]
    fn extract(req: &HttpRequest<S>) -> Self::Result {
        let req = req.clone();
        result(serde_urlencoded::from_str::<T>(req.query_string())
               .map_err(|e| e.into())
               .map(|item| Query{ item, req}))
    }
}

macro_rules! unsupported_type {
    ($trait_fn:ident, $name:expr) => {
        fn $trait_fn<V>(self, _: V) -> Result<V::Value, Self::Error>
            where V: Visitor<'de>
        {
            Err(de::value::Error::custom(concat!("unsupported type: ", $name)))
        }
    };
}

pub struct PathExtractor<'de, S: 'de> {
    req: &'de HttpRequest<S>
}

impl<'de, S: 'de> Deserializer<'de> for PathExtractor<'de, S>
{
    type Error = de::value::Error;

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_map(de::value::MapDeserializer::new(
            self.req.match_info().iter().map(|&(ref k, ref v)| (k.as_ref(), v.as_ref()))))
    }

    fn deserialize_struct<V>(self, _: &'static str, _: &'static [&'static str], visitor: V)
                             -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(self, _: &'static str, visitor: V)
                                  -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(self, _: &'static str, visitor: V)
                                     -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        if self.req.match_info().len() < len {
            Err(de::value::Error::custom(
                format!("wrong number of parameters: {} expected {}",
                        self.req.match_info().len(), len).as_str()))
        } else {
            visitor.visit_seq(de::value::SeqDeserializer::new(
                self.req.match_info().iter().map(|&(_, ref v)| v.as_ref())))
        }
    }

    fn deserialize_tuple_struct<V>(self, _: &'static str, _: usize, visitor: V)
                                   -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        visitor.visit_seq(de::value::SeqDeserializer::new(
            self.req.match_info().iter().map(|&(_, ref v)| v.as_ref())))
    }

    fn deserialize_enum<V>(self, _: &'static str, _: &'static [&'static str], _: V)
                           -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        Err(de::value::Error::custom("unsupported type: enum"))
    }

    unsupported_type!(deserialize_any, "'any'");
    unsupported_type!(deserialize_bool, "bool");
    unsupported_type!(deserialize_i8, "i8");
    unsupported_type!(deserialize_i16, "i16");
    unsupported_type!(deserialize_i32, "i32");
    unsupported_type!(deserialize_i64, "i64");
    unsupported_type!(deserialize_u8, "u8");
    unsupported_type!(deserialize_u16, "u16");
    unsupported_type!(deserialize_u32, "u32");
    unsupported_type!(deserialize_u64, "u64");
    unsupported_type!(deserialize_f32, "f32");
    unsupported_type!(deserialize_f64, "f64");
    unsupported_type!(deserialize_char, "char");
    unsupported_type!(deserialize_str, "str");
    unsupported_type!(deserialize_string, "String");
    unsupported_type!(deserialize_bytes, "bytes");
    unsupported_type!(deserialize_byte_buf, "byte buf");
    unsupported_type!(deserialize_option, "Option<T>");
    unsupported_type!(deserialize_seq, "sequence");
    unsupported_type!(deserialize_identifier, "identifier");
    unsupported_type!(deserialize_ignored_any, "ignored_any");
}

#[cfg(test)]
mod tests {
    use futures::Async;
    use super::*;
    use router::{Router, Pattern};
    use resource::Resource;
    use test::TestRequest;
    use server::ServerSettings;

    #[derive(Deserialize)]
    struct MyStruct {
        key: String,
        value: String,
    }

    #[derive(Deserialize)]
    struct Id {
        id: String,
    }

    #[test]
    fn test_request_extract() {
        let mut req = TestRequest::with_uri("/name/user1/?id=test").finish();

        let mut resource = Resource::<()>::default();
        resource.name("index");
        let mut routes = Vec::new();
        routes.push((Pattern::new("index", "/{key}/{value}/"), Some(resource)));
        let (router, _) = Router::new("", ServerSettings::default(), routes);
        assert!(router.recognize(&mut req).is_some());

        match Path::<MyStruct, _>::extract(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.key, "name");
                assert_eq!(s.value, "user1");
            },
            _ => unreachable!(),
        }

        match Path::<(String, String), _>::extract(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.0, "name");
                assert_eq!(s.1, "user1");
            },
            _ => unreachable!(),
        }

        match Query::<Id, _>::extract(&req).poll().unwrap() {
            Async::Ready(s) => {
                assert_eq!(s.id, "test");
            },
            _ => unreachable!(),
        }
    }
}
