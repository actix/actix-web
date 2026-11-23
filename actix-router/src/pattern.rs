/// One or many patterns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Patterns {
    Single(String),
    List(Vec<String>),
}

impl Patterns {
    pub fn is_empty(&self) -> bool {
        match self {
            Patterns::Single(_) => false,
            Patterns::List(pats) => pats.is_empty(),
        }
    }
}

/// Helper trait for type that could be converted to one or more path patterns.
pub trait IntoPatterns {
    fn patterns(&self) -> Patterns;
}

impl IntoPatterns for String {
    fn patterns(&self) -> Patterns {
        Patterns::Single(self.clone())
    }
}

impl IntoPatterns for &String {
    fn patterns(&self) -> Patterns {
        (*self).patterns()
    }
}

impl IntoPatterns for str {
    fn patterns(&self) -> Patterns {
        Patterns::Single(self.to_owned())
    }
}

impl IntoPatterns for &str {
    fn patterns(&self) -> Patterns {
        (*self).patterns()
    }
}

impl IntoPatterns for bytestring::ByteString {
    fn patterns(&self) -> Patterns {
        Patterns::Single(self.to_string())
    }
}

impl IntoPatterns for Patterns {
    fn patterns(&self) -> Patterns {
        self.clone()
    }
}

impl<T: AsRef<str>> IntoPatterns for Vec<T> {
    fn patterns(&self) -> Patterns {
        let mut patterns = self.iter().map(|v| v.as_ref().to_owned());

        match patterns.size_hint() {
            (1, _) => Patterns::Single(patterns.next().unwrap()),
            _ => Patterns::List(patterns.collect()),
        }
    }
}

macro_rules! array_patterns_single (($tp:ty) => {
    impl IntoPatterns for [$tp; 1] {
        fn patterns(&self) -> Patterns {
            Patterns::Single(self[0].to_owned())
        }
    }
});

macro_rules! array_patterns_multiple (($tp:ty, $str_fn:expr, $($num:tt) +) => {
    // for each array length specified in space-separated $num
    $(
        impl IntoPatterns for [$tp; $num] {
            fn patterns(&self) -> Patterns {
                Patterns::List(self.iter().map($str_fn).collect())
            }
        }
    )+
});

array_patterns_single!(&str);
array_patterns_multiple!(&str, |&v| v.to_owned(), 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);

array_patterns_single!(String);
array_patterns_multiple!(String, |v| v.clone(), 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);

#[cfg(test)]
mod tests {
    use bytestring::ByteString;

    use super::*;

    #[test]
    fn empty_patterns_are_distinguished_from_single_patterns() {
        assert!(!Patterns::Single("/one".to_owned()).is_empty());
        assert!(Patterns::List(Vec::new()).is_empty());
        assert!(!Patterns::List(vec!["/one".to_owned()]).is_empty());
    }

    #[test]
    fn supported_inputs_convert_to_patterns() {
        let owned = "/owned".to_owned();
        assert_eq!(owned.patterns(), Patterns::Single("/owned".to_owned()));
        let owned_ref = &owned;
        assert_eq!(owned_ref.patterns(), Patterns::Single("/owned".to_owned()));

        let str_value: &str = "/str";
        assert_eq!(
            <str as IntoPatterns>::patterns(str_value),
            Patterns::Single("/str".to_owned())
        );
        let ref_value: &str = "/ref";
        assert_eq!(
            <&str as IntoPatterns>::patterns(&ref_value),
            Patterns::Single("/ref".to_owned())
        );

        let bytes = ByteString::from("/bytes");
        assert_eq!(bytes.patterns(), Patterns::Single("/bytes".to_owned()));

        let original = Patterns::List(vec!["/first".to_owned(), "/second".to_owned()]);
        assert_eq!(original.patterns(), original);

        assert_eq!(vec!["/one"].patterns(), Patterns::Single("/one".to_owned()));
        assert_eq!(
            vec!["/one", "/two"].patterns(),
            Patterns::List(vec!["/one".to_owned(), "/two".to_owned()])
        );
        assert_eq!(
            vec!["/one".to_owned()].patterns(),
            Patterns::Single("/one".to_owned())
        );

        assert_eq!(["/one"].patterns(), Patterns::Single("/one".to_owned()));
        assert_eq!(
            ["/one", "/two"].patterns(),
            Patterns::List(vec!["/one".to_owned(), "/two".to_owned()])
        );
        assert_eq!(
            ["/one".to_owned()].patterns(),
            Patterns::Single("/one".to_owned())
        );
        assert_eq!(
            ["/one".to_owned(), "/two".to_owned()].patterns(),
            Patterns::List(vec!["/one".to_owned(), "/two".to_owned()])
        );
    }
}
