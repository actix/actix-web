macro_rules! common_header_test_module {
    ($id:ident, $tm:ident{$($tf:item)*}) => {
        #[cfg(test)]
        mod $tm {
            #![allow(unused_imports)]

            use ::core::str;

            use ::actix_http::{Method, test};
            use ::mime::*;

            use $crate::http::header::{self, *};
            use super::{$id as HeaderField, *};

            $($tf)*
        }
    }
}

#[cfg(test)]
macro_rules! common_header_test {
    ($id:ident, $raw:expr) => {
        #[test]
        fn $id() {
            use ::actix_http::test;

            let raw = $raw;
            let headers = raw.iter().map(|x| x.to_vec()).collect::<Vec<_>>();

            let mut req = test::TestRequest::default();

            for item in headers {
                req = req.append_header((HeaderField::name(), item)).take();
            }

            let req = req.finish();
            let value = HeaderField::parse(&req);

            let result = format!("{}", value.unwrap());
            let expected = ::std::string::String::from_utf8(raw[0].to_vec()).unwrap();

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

    ($id:ident, $raw:expr, $exp:expr) => {
        #[test]
        fn $id() {
            use actix_http::test;

            let headers = $raw.iter().map(|x| x.to_vec()).collect::<Vec<_>>();
            let mut req = test::TestRequest::default();

            for item in headers {
                req.append_header((HeaderField::name(), item));
            }

            let req = req.finish();
            let val = HeaderField::parse(&req);

            let exp: ::core::option::Option<HeaderField> = $exp;

            // test parsing
            assert_eq!(val.ok(), exp);

            // test formatting
            if let Some(exp) = exp {
                let raw = &($raw)[..];
                let mut iter = raw.iter().map(|b| str::from_utf8(&b[..]).unwrap());
                let mut joined = String::new();
                if let Some(s) = iter.next() {
                    joined.push_str(s);
                    for s in iter {
                        joined.push_str(", ");
                        joined.push_str(s);
                    }
                }
                assert_eq!(format!("{}", exp), joined);
            }
        }
    };
}

macro_rules! common_header {
    // TODO: these docs are wrong, there's no $n or $nn
    // $attrs:meta: Attributes associated with the header item (usually docs)
    // $id:ident: Identifier of the header
    // $n:expr: Lowercase name of the header
    // $nn:expr: Nice name of the header

    // List header, zero or more items
    ($(#[$attrs:meta])*($id:ident, $name:expr) => ($item:ty)*) => {
        $(#[$attrs])*
        #[derive(Debug, Clone, PartialEq, Eq, ::derive_more::Deref, ::derive_more::DerefMut)]
        pub struct $id(pub Vec<$item>);

        impl $crate::http::header::Header for $id {
            #[inline]
            fn name() -> $crate::http::header::HeaderName {
                $name
            }

            #[inline]
            fn parse<M: $crate::HttpMessage>(msg: &M) -> Result<Self, $crate::error::ParseError> {
                let headers = msg.headers().get_all(Self::name());
                $crate::http::header::from_comma_delimited(headers).map($id)
            }
        }

        impl ::core::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                $crate::http::header::fmt_comma_delimited(f, &self.0[..])
            }
        }

        impl $crate::http::header::TryIntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            #[inline]
            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                use ::core::fmt::Write;
                let mut writer = $crate::http::header::Writer::new();
                let _ = write!(&mut writer, "{}", self);
                $crate::http::header::HeaderValue::from_maybe_shared(writer.take())
            }
        }
    };

    // List header, one or more items
    ($(#[$attrs:meta])*($id:ident, $name:expr) => ($item:ty)+) => {
        $(#[$attrs])*
        #[derive(Debug, Clone, PartialEq, Eq, ::derive_more::Deref, ::derive_more::DerefMut)]
        pub struct $id(pub Vec<$item>);

        impl $crate::http::header::Header for $id {
            #[inline]
            fn name() -> $crate::http::header::HeaderName {
                $name
            }

            #[inline]
            fn parse<M: $crate::HttpMessage>(msg: &M) -> Result<Self, $crate::error::ParseError>{
                let headers = msg.headers().get_all(Self::name());

                $crate::http::header::from_comma_delimited(headers)
                    .and_then(|items| {
                        if items.is_empty() {
                            Err($crate::error::ParseError::Header)
                        } else {
                            Ok($id(items))
                        }
                    })
            }
        }

        impl ::core::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                $crate::http::header::fmt_comma_delimited(f, &self.0[..])
            }
        }

        impl $crate::http::header::TryIntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            #[inline]
            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                use ::core::fmt::Write;
                let mut writer = $crate::http::header::Writer::new();
                let _ = write!(&mut writer, "{}", self);
                $crate::http::header::HeaderValue::from_maybe_shared(writer.take())
            }
        }
    };

    // Single value header
    ($(#[$attrs:meta])*($id:ident, $name:expr) => [$value:ty]) => {
        $(#[$attrs])*
        #[derive(Debug, Clone, PartialEq, Eq, ::derive_more::Deref, ::derive_more::DerefMut)]
        pub struct $id(pub $value);

        impl $crate::http::header::Header for $id {
            #[inline]
            fn name() -> $crate::http::header::HeaderName {
                $name
            }

            #[inline]
            fn parse<M: $crate::HttpMessage>(msg: &M) -> Result<Self, $crate::error::ParseError> {
                let header = msg.headers().get(Self::name());
                $crate::http::header::from_one_raw_str(header).map($id)
            }
        }

        impl ::core::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        impl $crate::http::header::TryIntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            #[inline]
            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                self.0.try_into_value()
            }
        }
    };

    // List header, one or more items with "*" option
    ($(#[$attrs:meta])*($id:ident, $name:expr) => {Any / ($item:ty)+}) => {
        $(#[$attrs])*
        #[derive(Clone, Debug, PartialEq, Eq)]
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
            fn parse<M: $crate::HttpMessage>(msg: &M) -> Result<Self, $crate::error::ParseError> {
                let is_any = msg
                    .headers()
                    .get(Self::name())
                    .and_then(|hdr| hdr.to_str().ok())
                    .map(|hdr| hdr.trim() == "*");

                if let Some(true) = is_any {
                    Ok($id::Any)
                } else {
                    let headers = msg.headers().get_all(Self::name());
                    Ok($id::Items($crate::http::header::from_comma_delimited(headers)?))
                }
            }
        }

        impl ::core::fmt::Display for $id {
            #[inline]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match *self {
                    $id::Any => f.write_str("*"),
                    $id::Items(ref fields) =>
                        $crate::http::header::fmt_comma_delimited(f, &fields[..])
                }
            }
        }

        impl $crate::http::header::TryIntoHeaderValue for $id {
            type Error = $crate::http::header::InvalidHeaderValue;

            #[inline]
            fn try_into_value(self) -> Result<$crate::http::header::HeaderValue, Self::Error> {
                use ::core::fmt::Write;
                let mut writer = $crate::http::header::Writer::new();
                let _ = write!(&mut writer, "{}", self);
                $crate::http::header::HeaderValue::from_maybe_shared(writer.take())
            }
        }
    };

    // optional test module
    ($(#[$attrs:meta])*($id:ident, $name:expr) => ($item:ty)* $tm:ident{$($tf:item)*}) => {
        crate::http::header::common_header! {
            $(#[$attrs])*
            ($id, $name) => ($item)*
        }

        crate::http::header::common_header_test_module! { $id, $tm { $($tf)* }}
    };
    ($(#[$attrs:meta])*($id:ident, $n:expr) => ($item:ty)+ $tm:ident{$($tf:item)*}) => {
        crate::http::header::common_header! {
            $(#[$attrs])*
            ($id, $n) => ($item)+
        }

        crate::http::header::common_header_test_module! { $id, $tm { $($tf)* }}
    };
    ($(#[$attrs:meta])*($id:ident, $name:expr) => [$item:ty] $tm:ident{$($tf:item)*}) => {
        crate::http::header::common_header! {
            $(#[$attrs])* ($id, $name) => [$item]
        }

        crate::http::header::common_header_test_module! { $id, $tm { $($tf)* }}
    };
    ($(#[$attrs:meta])*($id:ident, $name:expr) => {Any / ($item:ty)+} $tm:ident{$($tf:item)*}) => {
        crate::http::header::common_header! {
            $(#[$attrs])*
            ($id, $name) => {Any / ($item)+}
        }

        crate::http::header::common_header_test_module! { $id, $tm { $($tf)* }}
    };
}

pub(crate) use common_header;
#[cfg(test)]
pub(crate) use common_header_test;
pub(crate) use common_header_test_module;
