#[doc(hidden)]
#[macro_export]
macro_rules! __common_header_deref {
    ($from:ty => $to:ty) => {
        impl ::std::ops::Deref for $from {
            type Target = $to;

            #[inline]
            fn deref(&self) -> &$to {
                &self.0
            }
        }

        impl ::std::ops::DerefMut for $from {
            #[inline]
            fn deref_mut(&mut self) -> &mut $to {
                &mut self.0
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __common_header_test_module {
    ($id:ident, $tm:ident{$($tf:item)*}) => {
        #[allow(unused_imports)]
        #[cfg(test)]
        mod $tm {
            use std::str;
            use actix_http::http::Method;
            use mime::*;
            use $crate::http::header::*;
            use super::$id as HeaderField;
            $($tf)*
        }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __common_header_test {
    ($id:ident, $raw:expr) => {
        #[test]
        fn $id() {
            use actix_http::test;

            let raw = $raw;
            let a: Vec<Vec<u8>> = raw.iter().map(|x| x.to_vec()).collect();
            let mut req = test::TestRequest::default();
            for item in a {
                req = req.insert_header((HeaderField::name(), item)).take();
            }
            let req = req.finish();
            let value = HeaderField::parse(&req);
            let result = format!("{}", value.unwrap());
            let expected = String::from_utf8(raw[0].to_vec()).unwrap();
            let result_cmp: Vec<String> = result
                .to_ascii_lowercase()
                .split(' ')
                .map(|x| x.to_owned())
                .collect();
            let expected_cmp: Vec<String> = expected
                .to_ascii_lowercase()
                .split(' ')
                .map(|x| x.to_owned())
                .collect();
            assert_eq!(result_cmp.concat(), expected_cmp.concat());
        }
    };
    ($id:ident, $raw:expr, $typed:expr) => {
        #[test]
        fn $id() {
            use actix_http::test;

            let a: Vec<Vec<u8>> = $raw.iter().map(|x| x.to_vec()).collect();
            let mut req = test::TestRequest::default();
            for item in a {
                req.insert_header((HeaderField::name(), item));
            }
            let req = req.finish();
            let val = HeaderField::parse(&req);
            let typed: Option<HeaderField> = $typed;
            // Test parsing
            assert_eq!(val.ok(), typed);
            // Test formatting
            if typed.is_some() {
                let raw = &($raw)[..];
                let mut iter = raw.iter().map(|b| str::from_utf8(&b[..]).unwrap());
                let mut joined = String::new();
                joined.push_str(iter.next().unwrap());
                for s in iter {
                    joined.push_str(", ");
                    joined.push_str(s);
                }
                assert_eq!(format!("{}", typed.unwrap()), joined);
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __define_common_header {
    // $a:meta: Attributes associated with the header item (usually docs)
    // $id:ident: Identifier of the header
    // $n:expr: Lowercase name of the header
    // $nn:expr: Nice name of the header

    // List header, zero or more items
    ($(#[$a:meta])*($id:ident, $name:expr) => ($item:ty)*) => {
        $(#[$a])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $id(pub Vec<$item>);
        crate::__common_header_deref!($id => Vec<$item>);
        impl $crate::http::header::Header for $id {
            #[inline]
            fn name() -> $crate::http::header::HeaderName {
                $name
            }
            #[inline]
            fn parse<T>(msg: &T) -> Result<Self, $crate::error::ParseError>
                where T: $crate::HttpMessage
            {
                $crate::http::header::from_comma_delimited(
                    msg.headers().get_all(Self::name())).map($id)
            }
        }
        impl std::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                $crate::http::header::fmt_comma_delimited(f, &self.0[..])
            }
        }
        impl $crate::http::header::IntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                use std::fmt::Write;
                let mut writer = $crate::http::header::Writer::new();
                let _ = write!(&mut writer, "{}", self);
                $crate::http::header::HeaderValue::from_maybe_shared(writer.take())
            }
        }
    };
    // List header, one or more items
    ($(#[$a:meta])*($id:ident, $name:expr) => ($item:ty)+) => {
        $(#[$a])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $id(pub Vec<$item>);
        crate::__common_header_deref!($id => Vec<$item>);
        impl $crate::http::header::Header for $id {
            #[inline]
            fn name() -> $crate::http::header::HeaderName {
                $name
            }
            #[inline]
            fn parse<T>(msg: &T) -> Result<Self, $crate::error::ParseError>
                where T: $crate::HttpMessage
            {
                $crate::http::header::from_comma_delimited(
                    msg.headers().get_all(Self::name())).map($id)
            }
        }
        impl std::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                $crate::http::header::fmt_comma_delimited(f, &self.0[..])
            }
        }
        impl $crate::http::header::IntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                use std::fmt::Write;
                let mut writer = $crate::http::header::Writer::new();
                let _ = write!(&mut writer, "{}", self);
                $crate::http::header::HeaderValue::from_maybe_shared(writer.take())
            }
        }
    };
    // Single value header
    ($(#[$a:meta])*($id:ident, $name:expr) => [$value:ty]) => {
        $(#[$a])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $id(pub $value);
        crate::__common_header_deref!($id => $value);
        impl $crate::http::header::Header for $id {
            #[inline]
            fn name() -> $crate::http::header::HeaderName {
                $name
            }
            #[inline]
            fn parse<T>(msg: &T) -> Result<Self, $crate::error::ParseError>
                where T: $crate::HttpMessage
            {
                $crate::http::header::from_one_raw_str(
                    msg.headers().get(Self::name())).map($id)
            }
        }
        impl std::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
        impl $crate::http::header::IntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                self.0.try_into_value()
            }
        }
    };
    // List header, one or more items with "*" option
    ($(#[$a:meta])*($id:ident, $name:expr) => {Any / ($item:ty)+}) => {
        $(#[$a])*
        #[derive(Clone, Debug, PartialEq)]
        pub enum $id {
            /// Any value is a match
            Any,
            /// Only the listed items are a match
            Items(Vec<$item>),
        }
        impl $crate::http::header::Header for $id {
            #[inline]
            fn name() -> $crate::http::header::HeaderName {
                $name
            }
            #[inline]
            fn parse<T>(msg: &T) -> Result<Self, $crate::error::ParseError>
                where T: $crate::HttpMessage
            {
                let any = msg.headers().get(Self::name()).and_then(|hdr| {
                    hdr.to_str().ok().and_then(|hdr| Some(hdr.trim() == "*"))});

                if let Some(true) = any {
                    Ok($id::Any)
                } else {
                    Ok($id::Items(
                        $crate::http::header::from_comma_delimited(
                            msg.headers().get_all(Self::name()))?))
                }
            }
        }
        impl std::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match *self {
                    $id::Any => f.write_str("*"),
                    $id::Items(ref fields) => $crate::http::header::fmt_comma_delimited(
                        f, &fields[..])
                }
            }
        }
        impl $crate::http::header::IntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                use std::fmt::Write;
                let mut writer = $crate::http::header::Writer::new();
                let _ = write!(&mut writer, "{}", self);
                $crate::http::header::HeaderValue::from_maybe_shared(writer.take())
            }
        }
    };

    // optional test module
    ($(#[$a:meta])*($id:ident, $name:expr) => ($item:ty)* $tm:ident{$($tf:item)*}) => {
        crate::__define_common_header! {
            $(#[$a])*
            ($id, $name) => ($item)*
        }

        crate::__common_header_test_module! { $id, $tm { $($tf)* }}
    };
    ($(#[$a:meta])*($id:ident, $n:expr) => ($item:ty)+ $tm:ident{$($tf:item)*}) => {
        crate::__define_common_header! {
            $(#[$a])*
            ($id, $n) => ($item)+
        }

        crate::__common_header_test_module! { $id, $tm { $($tf)* }}
    };
    ($(#[$a:meta])*($id:ident, $name:expr) => [$item:ty] $tm:ident{$($tf:item)*}) => {
        crate::__define_common_header! {
            $(#[$a])* ($id, $name) => [$item]
        }

        crate::__common_header_test_module! { $id, $tm { $($tf)* }}
    };
    ($(#[$a:meta])*($id:ident, $name:expr) => {Any / ($item:ty)+} $tm:ident{$($tf:item)*}) => {
        crate::__define_common_header! {
            $(#[$a])*
            ($id, $name) => {Any / ($item)+}
        }

        crate::__common_header_test_module! { $id, $tm { $($tf)* }}
    };
}
