//! `x-www-form-urlencoded` meets Serde

extern crate dtoa;
extern crate itoa;

pub mod de;
pub mod ser;

#[doc(inline)]
pub use self::de::{from_bytes, from_reader, from_str, Deserializer};
#[doc(inline)]
pub use self::ser::{to_string, Serializer};

#[cfg(test)]
mod tests {
    #[test]
    fn deserialize_bytes() {
        let result = vec![("first".to_owned(), 23), ("last".to_owned(), 42)];

        assert_eq!(super::from_bytes(b"first=23&last=42"), Ok(result));
    }

    #[test]
    fn deserialize_str() {
        let result = vec![("first".to_owned(), 23), ("last".to_owned(), 42)];

        assert_eq!(super::from_str("first=23&last=42"), Ok(result));
    }

    #[test]
    fn deserialize_reader() {
        let result = vec![("first".to_owned(), 23), ("last".to_owned(), 42)];

        assert_eq!(super::from_reader(b"first=23&last=42" as &[_]), Ok(result));
    }

    #[test]
    fn deserialize_option() {
        let result = vec![
            ("first".to_owned(), Some(23)),
            ("last".to_owned(), Some(42)),
        ];
        assert_eq!(super::from_str("first=23&last=42"), Ok(result));
    }

    #[test]
    fn deserialize_unit() {
        assert_eq!(super::from_str(""), Ok(()));
        assert_eq!(super::from_str("&"), Ok(()));
        assert_eq!(super::from_str("&&"), Ok(()));
        assert!(super::from_str::<()>("first=23").is_err());
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    enum X {
        A,
        B,
        C,
    }

    #[test]
    fn deserialize_unit_enum() {
        let result = vec![
            ("one".to_owned(), X::A),
            ("two".to_owned(), X::B),
            ("three".to_owned(), X::C),
        ];

        assert_eq!(super::from_str("one=A&two=B&three=C"), Ok(result));
    }

    #[test]
    fn serialize_option_map_int() {
        let params = &[("first", Some(23)), ("middle", None), ("last", Some(42))];

        assert_eq!(super::to_string(params), Ok("first=23&last=42".to_owned()));
    }

    #[test]
    fn serialize_option_map_string() {
        let params = &[
            ("first", Some("hello")),
            ("middle", None),
            ("last", Some("world")),
        ];

        assert_eq!(
            super::to_string(params),
            Ok("first=hello&last=world".to_owned())
        );
    }

    #[test]
    fn serialize_option_map_bool() {
        let params = &[("one", Some(true)), ("two", Some(false))];

        assert_eq!(
            super::to_string(params),
            Ok("one=true&two=false".to_owned())
        );
    }

    #[test]
    fn serialize_map_bool() {
        let params = &[("one", true), ("two", false)];

        assert_eq!(
            super::to_string(params),
            Ok("one=true&two=false".to_owned())
        );
    }

    #[test]
    fn serialize_unit_enum() {
        let params = &[("one", X::A), ("two", X::B), ("three", X::C)];
        assert_eq!(
            super::to_string(params),
            Ok("one=A&two=B&three=C".to_owned())
        );
    }
}
