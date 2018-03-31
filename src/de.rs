use std::slice::Iter;
use std::borrow::Cow;
use std::ops::{Deref, DerefMut};

use serde_urlencoded;
use serde::de::{self, Deserializer, DeserializeOwned, Visitor, Error as DeError};
use futures::future::{FutureResult, result};

use error::Error;
use handler::FromRequest;
use httprequest::HttpRequest;

/// Extract typed information from the request's path.
///
/// ## Example
///
/// ```rust
/// # extern crate bytes;
/// # extern crate actix_web;
/// # extern crate futures;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{Application, Path, Result, http};
///
/// /// extract path info from "/{username}/{count}/?index.html" url
/// /// {username} - deserializes to a String
/// /// {count} -  - deserializes to a u32
/// fn index(info: Path<(String, u32)>) -> Result<String> {
///     Ok(format!("Welcome {}! {}", info.0, info.1))
/// }
///
/// fn main() {
///     let app = Application::new().resource(
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
/// use actix_web::{Application, Path, Result, http};
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
///     let app = Application::new().resource(
///        "/{username}/index.html",                // <- define path parameters
///        |r| r.method(http::Method::GET).with(index));  // <- use `with` extractor
/// }
/// ```
pub struct Path<T>{
    inner: T
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
    /// Deconstruct to a inner value
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
        result(de::Deserialize::deserialize(PathDeserializer{req: &req})
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
/// use actix_web::{Application, Query, http};
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
///     let app = Application::new().resource(
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

macro_rules! unsupported_type {
    ($trait_fn:ident, $name:expr) => {
        fn $trait_fn<V>(self, _: V) -> Result<V::Value, Self::Error>
            where V: Visitor<'de>
        {
            Err(de::value::Error::custom(concat!("unsupported type: ", $name)))
        }
    };
}

pub struct PathDeserializer<'de, S: 'de> {
    req: &'de HttpRequest<S>
}

impl<'de, S: 'de> Deserializer<'de> for PathDeserializer<'de, S>
{
    type Error = de::value::Error;

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_map(ParamsDeserializer{
            params: self.req.match_info().iter(),
            current: None,
        })
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
            visitor.visit_seq(ParamsSeq{params: self.req.match_info().iter()})
        }
    }

    fn deserialize_tuple_struct<V>(self, _: &'static str, _: usize, visitor: V)
                                   -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        visitor.visit_seq(ParamsSeq{params: self.req.match_info().iter()})
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

struct ParamsDeserializer<'de> {
    params: Iter<'de, (Cow<'de, str>, Cow<'de, str>)>,
    current: Option<(&'de str, &'de str)>,
}

impl<'de> de::MapAccess<'de> for ParamsDeserializer<'de>
{
    type Error = de::value::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
        where K: de::DeserializeSeed<'de>,
    {
        self.current = self.params.next().map(|&(ref k, ref v)| (k.as_ref(), v.as_ref()));
        match self.current {
            Some((key, _)) => Ok(Some(seed.deserialize(Key{key})?)),
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
        where V: de::DeserializeSeed<'de>,
    {
        if let Some((_, value)) = self.current.take() {
            seed.deserialize(Value { value })
        } else {
            Err(de::value::Error::custom("unexpected item"))
        }
    }
}

struct Key<'de> {
    key: &'de str,
}

impl<'de> Deserializer<'de> for Key<'de> {
    type Error = de::value::Error;

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_str(self.key)
    }

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        Err(de::value::Error::custom("Unexpected"))
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
            byte_buf option unit unit_struct newtype_struct seq tuple
            tuple_struct map struct enum ignored_any
    }
}

macro_rules! parse_value {
    ($trait_fn:ident, $visitor_fn:ident, $tp:tt) => {
        fn $trait_fn<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where V: Visitor<'de>
        {
            let v = self.value.parse().map_err(
                |_| de::value::Error::custom(
                    format!("can not parse {:?} to a {}", self.value, $tp)))?;
            visitor.$visitor_fn(v)
        }
    }
}

struct Value<'de> {
    value: &'de str,
}

impl<'de> Deserializer<'de> for Value<'de>
{
    type Error = de::value::Error;

    parse_value!(deserialize_bool, visit_bool, "bool");
    parse_value!(deserialize_i8, visit_i8, "i8");
    parse_value!(deserialize_i16, visit_i16, "i16");
    parse_value!(deserialize_i32, visit_i32, "i16");
    parse_value!(deserialize_i64, visit_i64, "i64");
    parse_value!(deserialize_u8, visit_u8, "u8");
    parse_value!(deserialize_u16, visit_u16, "u16");
    parse_value!(deserialize_u32, visit_u32, "u32");
    parse_value!(deserialize_u64, visit_u64, "u64");
    parse_value!(deserialize_f32, visit_f32, "f32");
    parse_value!(deserialize_f64, visit_f64, "f64");
    parse_value!(deserialize_string, visit_string, "String");
    parse_value!(deserialize_byte_buf, visit_string, "String");
    parse_value!(deserialize_char, visit_char, "char");

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(
        self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        visitor.visit_unit()
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_borrowed_bytes(self.value.as_bytes())
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_enum<V>(self, _: &'static str, _: &'static [&'static str], visitor: V)
                           -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_enum(ValueEnum {value: self.value})
    }

    fn deserialize_newtype_struct<V>(self, _: &'static str, visitor: V)
                                     -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V>(self, _: usize, _: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        Err(de::value::Error::custom("unsupported type: tuple"))
    }

    fn deserialize_struct<V>(self, _: &'static str, _: &'static [&'static str], _: V)
                             -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        Err(de::value::Error::custom("unsupported type: struct"))
    }

    fn deserialize_tuple_struct<V>(self, _: &'static str, _: usize, _: V)
                                   -> Result<V::Value, Self::Error>
        where V: Visitor<'de>
    {
        Err(de::value::Error::custom("unsupported type: tuple struct"))
    }

    unsupported_type!(deserialize_any, "");
    unsupported_type!(deserialize_seq, "seq");
    unsupported_type!(deserialize_map, "map");
    unsupported_type!(deserialize_identifier, "identifier");
}

struct ParamsSeq<'de> {
    params: Iter<'de, (Cow<'de, str>, Cow<'de, str>)>,
}

impl<'de> de::SeqAccess<'de> for ParamsSeq<'de>
{
    type Error = de::value::Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
        where T: de::DeserializeSeed<'de>,
    {
        match self.params.next() {
            Some(item) => Ok(Some(seed.deserialize(Value { value: item.1.as_ref() })?)),
            None => Ok(None),
        }
    }
}

struct ValueEnum<'de> {
    value: &'de str,
}

impl<'de> de::EnumAccess<'de> for ValueEnum<'de> {
    type Error = de::value::Error;
    type Variant = UnitVariant;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
        where V: de::DeserializeSeed<'de>,
    {
        Ok((seed.deserialize(Key { key: self.value })?, UnitVariant))
    }
}

struct UnitVariant;

impl<'de> de::VariantAccess<'de> for UnitVariant {
    type Error = de::value::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<T>(self, _seed: T) -> Result<T::Value, Self::Error>
        where T: de::DeserializeSeed<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }

    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }

    fn struct_variant<V>(self, _: &'static [&'static str], _: V)
                         -> Result<V::Value, Self::Error>
        where V: Visitor<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }
}

#[cfg(test)]
mod tests {
    use futures::{Async, Future};
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

    #[derive(Deserialize)]
    struct Test2 {
        key: String,
        value: u32,
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
                assert_eq!(s.key, "name");
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
    }
}
