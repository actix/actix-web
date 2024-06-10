use std::borrow::Cow;

use serde::{
    de::{self, Deserializer, Error as DeError, Visitor},
    forward_to_deserialize_any,
};

use crate::{
    path::{Path, PathIter},
    Quoter, ResourcePath,
};

thread_local! {
    static FULL_QUOTER: Quoter = Quoter::new(b"", b"");
}

macro_rules! unsupported_type {
    ($trait_fn:ident, $name:expr) => {
        fn $trait_fn<V>(self, _: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            Err(de::Error::custom(concat!("unsupported type: ", $name)))
        }
    };
}

macro_rules! parse_single_value {
    ($trait_fn:ident) => {
        fn $trait_fn<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            if self.path.segment_count() != 1 {
                Err(de::value::Error::custom(
                    format!(
                        "wrong number of parameters: {} expected 1",
                        self.path.segment_count()
                    )
                    .as_str(),
                ))
            } else {
                Value {
                    value: &self.path[0],
                }
                .$trait_fn(visitor)
            }
        }
    };
}

macro_rules! parse_value {
    ($trait_fn:ident, $visit_fn:ident, $tp:tt) => {
        fn $trait_fn<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let decoded = FULL_QUOTER
                .with(|q| q.requote_str_lossy(self.value))
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed(self.value));

            let v = decoded.parse().map_err(|_| {
                de::value::Error::custom(format!("can not parse {:?} to a {}", self.value, $tp))
            })?;

            visitor.$visit_fn(v)
        }
    };
}

pub struct PathDeserializer<'de, T: ResourcePath> {
    path: &'de Path<T>,
}

impl<'de, T: ResourcePath + 'de> PathDeserializer<'de, T> {
    pub fn new(path: &'de Path<T>) -> Self {
        PathDeserializer { path }
    }
}

impl<'de, T: ResourcePath + 'de> Deserializer<'de> for PathDeserializer<'de, T> {
    type Error = de::value::Error;

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(ParamsDeserializer {
            params: self.path.iter(),
            current: None,
        })
    }

    fn deserialize_struct<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.path.segment_count() < len {
            Err(de::value::Error::custom(
                format!(
                    "wrong number of parameters: {} expected {}",
                    self.path.segment_count(),
                    len
                )
                .as_str(),
            ))
        } else {
            visitor.visit_seq(ParamsSeq {
                params: self.path.iter(),
            })
        }
    }

    fn deserialize_tuple_struct<V>(
        self,
        _: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.path.segment_count() < len {
            Err(de::value::Error::custom(
                format!(
                    "wrong number of parameters: {} expected {}",
                    self.path.segment_count(),
                    len
                )
                .as_str(),
            ))
        } else {
            visitor.visit_seq(ParamsSeq {
                params: self.path.iter(),
            })
        }
    }

    fn deserialize_enum<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.path.is_empty() {
            Err(de::value::Error::custom("expected at least one parameters"))
        } else {
            visitor.visit_enum(ValueEnum {
                value: &self.path[0],
            })
        }
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(ParamsSeq {
            params: self.path.iter(),
        })
    }

    unsupported_type!(deserialize_any, "'any'");
    unsupported_type!(deserialize_option, "Option<T>");
    unsupported_type!(deserialize_identifier, "identifier");
    unsupported_type!(deserialize_ignored_any, "ignored_any");

    parse_single_value!(deserialize_bool);
    parse_single_value!(deserialize_i8);
    parse_single_value!(deserialize_i16);
    parse_single_value!(deserialize_i32);
    parse_single_value!(deserialize_i64);
    parse_single_value!(deserialize_u8);
    parse_single_value!(deserialize_u16);
    parse_single_value!(deserialize_u32);
    parse_single_value!(deserialize_u64);
    parse_single_value!(deserialize_f32);
    parse_single_value!(deserialize_f64);
    parse_single_value!(deserialize_str);
    parse_single_value!(deserialize_string);
    parse_single_value!(deserialize_bytes);
    parse_single_value!(deserialize_byte_buf);
    parse_single_value!(deserialize_char);
}

struct ParamsDeserializer<'de, T: ResourcePath> {
    params: PathIter<'de, T>,
    current: Option<(&'de str, &'de str)>,
}

impl<'de, T: ResourcePath> de::MapAccess<'de> for ParamsDeserializer<'de, T> {
    type Error = de::value::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        self.current = self.params.next().map(|ref item| (item.0, item.1));
        match self.current {
            Some((key, _)) => Ok(Some(seed.deserialize(Key { key })?)),
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: de::DeserializeSeed<'de>,
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
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.key)
    }

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("Unexpected"))
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
            byte_buf option unit unit_struct newtype_struct seq tuple
            tuple_struct map struct enum ignored_any
    }
}

struct Value<'de> {
    value: &'de str,
}

impl<'de> Deserializer<'de> for Value<'de> {
    type Error = de::value::Error;

    parse_value!(deserialize_bool, visit_bool, "bool");
    parse_value!(deserialize_i8, visit_i8, "i8");
    parse_value!(deserialize_i16, visit_i16, "i16");
    parse_value!(deserialize_i32, visit_i32, "i32");
    parse_value!(deserialize_i64, visit_i64, "i64");
    parse_value!(deserialize_u8, visit_u8, "u8");
    parse_value!(deserialize_u16, visit_u16, "u16");
    parse_value!(deserialize_u32, visit_u32, "u32");
    parse_value!(deserialize_u64, visit_u64, "u64");
    parse_value!(deserialize_f32, visit_f32, "f32");
    parse_value!(deserialize_f64, visit_f64, "f64");
    parse_value!(deserialize_char, visit_char, "char");

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match FULL_QUOTER.with(|q| q.requote_str_lossy(self.value)) {
            Some(s) => visitor.visit_string(s),
            None => visitor.visit_borrowed_str(self.value),
        }
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match FULL_QUOTER.with(|q| q.requote_str_lossy(self.value)) {
            Some(s) => visitor.visit_byte_buf(s.into()),
            None => visitor.visit_borrowed_bytes(self.value.as_bytes()),
        }
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_enum<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_enum(ValueEnum { value: self.value })
    }

    fn deserialize_newtype_struct<V>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V>(self, _: usize, _: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("unsupported type: tuple"))
    }

    fn deserialize_struct<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("unsupported type: struct"))
    }

    fn deserialize_tuple_struct<V>(
        self,
        _: &'static str,
        _: usize,
        _: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("unsupported type: tuple struct"))
    }

    unsupported_type!(deserialize_any, "any");
    unsupported_type!(deserialize_seq, "seq");
    unsupported_type!(deserialize_map, "map");
    unsupported_type!(deserialize_identifier, "identifier");
}

struct ParamsSeq<'de, T: ResourcePath> {
    params: PathIter<'de, T>,
}

impl<'de, T: ResourcePath> de::SeqAccess<'de> for ParamsSeq<'de, T> {
    type Error = de::value::Error;

    fn next_element_seed<U>(&mut self, seed: U) -> Result<Option<U::Value>, Self::Error>
    where
        U: de::DeserializeSeed<'de>,
    {
        match self.params.next() {
            Some(item) => Ok(Some(seed.deserialize(Value { value: item.1 })?)),
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
    where
        V: de::DeserializeSeed<'de>,
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
    where
        T: de::DeserializeSeed<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }

    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }

    fn struct_variant<V>(self, _: &'static [&'static str], _: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::{router::Router, ResourceDef};

    #[derive(Deserialize)]
    struct MyStruct {
        key: String,
        value: String,
    }

    #[derive(Deserialize)]
    struct Id {
        _id: String,
    }

    #[derive(Debug, Deserialize)]
    struct Test1(String, u32);

    #[derive(Debug, Deserialize)]
    struct Test2 {
        key: String,
        value: u32,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum TestEnum {
        Val1,
        Val2,
    }

    #[derive(Debug, Deserialize)]
    struct Test3 {
        val: TestEnum,
    }

    #[test]
    fn test_request_extract() {
        let mut router = Router::<()>::build();
        router.path("/{key}/{value}/", ());
        let router = router.finish();

        let mut path = Path::new("/name/user1/");
        assert!(router.recognize(&mut path).is_some());

        let s: MyStruct = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(s.key, "name");
        assert_eq!(s.value, "user1");

        let s: (String, String) =
            de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, "user1");

        let mut router = Router::<()>::build();
        router.path("/{key}/{value}/", ());
        let router = router.finish();

        let mut path = Path::new("/name/32/");
        assert!(router.recognize(&mut path).is_some());

        let s: Test1 = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, 32);

        let s: Test2 = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(s.key, "name");
        assert_eq!(s.value, 32);

        let s: (String, u8) = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, 32);

        let res: Vec<String> = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(res[0], "name".to_owned());
        assert_eq!(res[1], "32".to_owned());
    }

    #[test]
    fn test_extract_path_single() {
        let mut router = Router::<()>::build();
        router.path("/{value}/", ());
        let router = router.finish();

        let mut path = Path::new("/32/");
        assert!(router.recognize(&mut path).is_some());
        let i: i8 = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(i, 32);
    }

    #[test]
    fn test_extract_enum() {
        let mut router = Router::<()>::build();
        router.path("/{val}/", ());
        let router = router.finish();

        let mut path = Path::new("/val1/");
        assert!(router.recognize(&mut path).is_some());
        let i: TestEnum = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(i, TestEnum::Val1);

        let mut router = Router::<()>::build();
        router.path("/{val1}/{val2}/", ());
        let router = router.finish();

        let mut path = Path::new("/val1/val2/");
        assert!(router.recognize(&mut path).is_some());
        let i: (TestEnum, TestEnum) =
            de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(i, (TestEnum::Val1, TestEnum::Val2));
    }

    #[test]
    fn test_extract_enum_value() {
        let mut router = Router::<()>::build();
        router.path("/{val}/", ());
        let router = router.finish();

        let mut path = Path::new("/val1/");
        assert!(router.recognize(&mut path).is_some());
        let i: Test3 = de::Deserialize::deserialize(PathDeserializer::new(&path)).unwrap();
        assert_eq!(i.val, TestEnum::Val1);

        let mut path = Path::new("/val3/");
        assert!(router.recognize(&mut path).is_some());
        let i: Result<Test3, de::value::Error> =
            de::Deserialize::deserialize(PathDeserializer::new(&path));
        assert!(i.is_err());
        assert!(format!("{:?}", i).contains("unknown variant"));
    }

    #[test]
    fn test_extract_errors() {
        let mut router = Router::<()>::build();
        router.path("/{value}/", ());
        let router = router.finish();

        let mut path = Path::new("/name/");
        assert!(router.recognize(&mut path).is_some());

        let s: Result<Test1, de::value::Error> =
            de::Deserialize::deserialize(PathDeserializer::new(&path));
        assert!(s.is_err());
        assert!(format!("{:?}", s).contains("wrong number of parameters"));

        let s: Result<Test2, de::value::Error> =
            de::Deserialize::deserialize(PathDeserializer::new(&path));
        assert!(s.is_err());
        assert!(format!("{:?}", s).contains("can not parse"));

        let s: Result<(String, String), de::value::Error> =
            de::Deserialize::deserialize(PathDeserializer::new(&path));
        assert!(s.is_err());
        assert!(format!("{:?}", s).contains("wrong number of parameters"));

        let s: Result<u32, de::value::Error> =
            de::Deserialize::deserialize(PathDeserializer::new(&path));
        assert!(s.is_err());
        assert!(format!("{:?}", s).contains("can not parse"));
    }

    #[test]
    fn deserialize_path_decode_string() {
        let rdef = ResourceDef::new("/{key}");

        let mut path = Path::new("/%25");
        rdef.capture_match_info(&mut path);
        let de = PathDeserializer::new(&path);
        let segment: String = serde::Deserialize::deserialize(de).unwrap();
        assert_eq!(segment, "%");

        let mut path = Path::new("/%2F");
        rdef.capture_match_info(&mut path);
        let de = PathDeserializer::new(&path);
        let segment: String = serde::Deserialize::deserialize(de).unwrap();
        assert_eq!(segment, "/")
    }

    #[test]
    fn deserialize_path_decode_seq() {
        let rdef = ResourceDef::new("/{key}/{value}");

        let mut path = Path::new("/%30%25/%30%2F");
        rdef.capture_match_info(&mut path);
        let de = PathDeserializer::new(&path);
        let segment: (String, String) = serde::Deserialize::deserialize(de).unwrap();
        assert_eq!(segment.0, "0%");
        assert_eq!(segment.1, "0/");
    }

    #[test]
    fn deserialize_path_decode_map() {
        #[derive(Deserialize)]
        struct Vals {
            key: String,
            value: String,
        }

        let rdef = ResourceDef::new("/{key}/{value}");

        let mut path = Path::new("/%25/%2F");
        rdef.capture_match_info(&mut path);
        let de = PathDeserializer::new(&path);
        let vals: Vals = serde::Deserialize::deserialize(de).unwrap();
        assert_eq!(vals.key, "%");
        assert_eq!(vals.value, "/");
    }

    #[test]
    fn deserialize_borrowed() {
        #[derive(Debug, Deserialize)]
        struct Params<'a> {
            val: &'a str,
        }

        let rdef = ResourceDef::new("/{val}");

        let mut path = Path::new("/X");
        rdef.capture_match_info(&mut path);
        let de = PathDeserializer::new(&path);
        let params: Params<'_> = serde::Deserialize::deserialize(de).unwrap();
        assert_eq!(params.val, "X");
        let de = PathDeserializer::new(&path);
        let params: &str = serde::Deserialize::deserialize(de).unwrap();
        assert_eq!(params, "X");

        let mut path = Path::new("/%2F");
        rdef.capture_match_info(&mut path);
        let de = PathDeserializer::new(&path);
        assert!(<Params<'_> as serde::Deserialize>::deserialize(de).is_err());
        let de = PathDeserializer::new(&path);
        assert!(<&str as serde::Deserialize>::deserialize(de).is_err());
    }

    // #[test]
    // fn test_extract_path_decode() {
    //     let mut router = Router::<()>::default();
    //     router.register_resource(Resource::new(ResourceDef::new("/{value}/")));

    //     macro_rules! test_single_value {
    //         ($value:expr, $expected:expr) => {{
    //             let req = TestRequest::with_uri($value).finish();
    //             let info = router.recognize(&req, &(), 0);
    //             let req = req.with_route_info(info);
    //             assert_eq!(
    //                 *Path::<String>::from_request(&req, &PathConfig::default()).unwrap(),
    //                 $expected
    //             );
    //         }};
    //     }

    //     test_single_value!("/%25/", "%");
    //     test_single_value!("/%40%C2%A3%24%25%5E%26%2B%3D/", "@£$%^&+=");
    //     test_single_value!("/%2B/", "+");
    //     test_single_value!("/%252B/", "%2B");
    //     test_single_value!("/%2F/", "/");
    //     test_single_value!("/%252F/", "%2F");
    //     test_single_value!(
    //         "/http%3A%2F%2Flocalhost%3A80%2Ffoo/",
    //         "http://localhost:80/foo"
    //     );
    //     test_single_value!("/%2Fvar%2Flog%2Fsyslog/", "/var/log/syslog");
    //     test_single_value!(
    //         "/http%3A%2F%2Flocalhost%3A80%2Ffile%2F%252Fvar%252Flog%252Fsyslog/",
    //         "http://localhost:80/file/%2Fvar%2Flog%2Fsyslog"
    //     );

    //     let req = TestRequest::with_uri("/%25/7/?id=test").finish();

    //     let mut router = Router::<()>::default();
    //     router.register_resource(Resource::new(ResourceDef::new("/{key}/{value}/")));
    //     let info = router.recognize(&req, &(), 0);
    //     let req = req.with_route_info(info);

    //     let s = Path::<Test2>::from_request(&req, &PathConfig::default()).unwrap();
    //     assert_eq!(s.key, "%");
    //     assert_eq!(s.value, 7);

    //     let s = Path::<(String, String)>::from_request(&req, &PathConfig::default()).unwrap();
    //     assert_eq!(s.0, "%");
    //     assert_eq!(s.1, "7");
    // }

    // #[test]
    // fn test_extract_path_no_decode() {
    //     let mut router = Router::<()>::default();
    //     router.register_resource(Resource::new(ResourceDef::new("/{value}/")));

    //     let req = TestRequest::with_uri("/%25/").finish();
    //     let info = router.recognize(&req, &(), 0);
    //     let req = req.with_route_info(info);
    //     assert_eq!(
    //         *Path::<String>::from_request(&req, &&PathConfig::default().disable_decoding())
    //             .unwrap(),
    //         "%25"
    //     );
    // }
}
