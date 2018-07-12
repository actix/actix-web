extern crate serde_urlencoded;

#[test]
fn serialize_option_map_int() {
    let params = &[("first", Some(23)), ("middle", None), ("last", Some(42))];

    assert_eq!(serde_urlencoded::to_string(params),
               Ok("first=23&last=42".to_owned()));
}

#[test]
fn serialize_option_map_string() {
    let params =
        &[("first", Some("hello")), ("middle", None), ("last", Some("world"))];

    assert_eq!(serde_urlencoded::to_string(params),
               Ok("first=hello&last=world".to_owned()));
}

#[test]
fn serialize_option_map_bool() {
    let params = &[("one", Some(true)), ("two", Some(false))];

    assert_eq!(serde_urlencoded::to_string(params),
               Ok("one=true&two=false".to_owned()));
}

#[test]
fn serialize_map_bool() {
    let params = &[("one", true), ("two", false)];

    assert_eq!(serde_urlencoded::to_string(params),
               Ok("one=true&two=false".to_owned()));
}
