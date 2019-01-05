use std::cmp::min;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use regex::{escape, Regex};

use crate::path::{Path, PathItem};
use crate::RequestPath;

const MAX_DYNAMIC_SEGMENTS: usize = 16;

/// Resource type describes an entry in resources table
///
/// Resource pattern can contain only 16 dynamic segments
#[derive(Clone, Debug)]
pub struct Pattern {
    tp: PatternType,
    pattern: String,
    elements: Vec<PatternElement>,
}

#[derive(Debug, Clone, PartialEq)]
enum PatternElement {
    Str(String),
    Var(String),
}

#[derive(Clone, Debug)]
enum PatternType {
    Static(String),
    Prefix(String),
    Dynamic(Regex, Vec<Rc<String>>, usize),
}

impl Pattern {
    /// Parse path pattern and create new `Pattern` instance.
    ///
    /// Panics if path pattern is wrong.
    pub fn new(path: &str) -> Self {
        Pattern::with_prefix(path, false)
    }

    /// Parse path pattern and create new `Pattern` instance.
    ///
    /// Use `prefix` type instead of `static`.
    ///
    /// Panics if path regex pattern is wrong.
    pub fn prefix(path: &str) -> Self {
        Pattern::with_prefix(path, true)
    }

    /// Parse path pattern and create new `Pattern` instance with custom prefix
    fn with_prefix(path: &str, for_prefix: bool) -> Self {
        let path = path.to_owned();
        let (pattern, elements, is_dynamic, len) = Pattern::parse(&path, for_prefix);

        let tp = if is_dynamic {
            let re = match Regex::new(&pattern) {
                Ok(re) => re,
                Err(err) => panic!("Wrong path pattern: \"{}\" {}", path, err),
            };
            // actix creates one router per thread
            let names = re
                .capture_names()
                .filter_map(|name| name.map(|name| Rc::new(name.to_owned())))
                .collect();
            PatternType::Dynamic(re, names, len)
        } else if for_prefix {
            PatternType::Prefix(pattern.clone())
        } else {
            PatternType::Static(pattern.clone())
        };

        Pattern {
            tp,
            elements,
            pattern: path.to_owned(),
        }
    }

    /// Path pattern of the resource
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Check if path matchs this pattern?
    pub fn is_match(&self, path: &str) -> bool {
        match self.tp {
            PatternType::Static(ref s) => s == path,
            PatternType::Dynamic(ref re, _, _) => re.is_match(path),
            PatternType::Prefix(ref s) => path.starts_with(s),
        }
    }

    /// Is the given path and parameters a match against this pattern?
    pub fn match_path<T: RequestPath>(&self, path: &mut Path<T>) -> bool {
        match self.tp {
            PatternType::Static(ref s) => {
                if s == path.path() {
                    path.skip(path.len() as u16);
                    true
                } else {
                    false
                }
            }
            PatternType::Dynamic(ref re, ref names, len) => {
                let mut idx = 0;
                let mut pos = 0;
                let mut segments: [PathItem; MAX_DYNAMIC_SEGMENTS] =
                    [PathItem::Static(""); MAX_DYNAMIC_SEGMENTS];

                if let Some(captures) = re.captures(path.path()) {
                    let mut passed = false;

                    for capture in captures.iter() {
                        if let Some(ref m) = capture {
                            if !passed {
                                passed = true;
                                continue;
                            }

                            segments[idx] = PathItem::Segment(m.start() as u16, m.end() as u16);
                            idx += 1;
                            pos = m.end();
                        }
                    }
                } else {
                    return false;
                }
                for idx in 0..idx {
                    path.add(names[idx].clone(), segments[idx]);
                }
                path.skip((pos + len) as u16);
                true
            }
            PatternType::Prefix(ref s) => {
                let rpath = path.path();
                let len = if s == rpath {
                    s.len()
                } else if rpath.starts_with(s)
                    && (s.ends_with('/') || rpath.split_at(s.len()).1.starts_with('/'))
                {
                    if s.ends_with('/') {
                        s.len() - 1
                    } else {
                        s.len()
                    }
                } else {
                    return false;
                };
                path.skip(min(rpath.len(), len) as u16);
                true
            }
        }
    }

    // /// Build resource path.
    // pub fn resource_path<U, I>(
    //     &self, path: &mut String, elements: &mut U,
    // ) -> Result<(), UrlGenerationError>
    // where
    //     U: Iterator<Item = I>,
    //     I: AsRef<str>,
    // {
    //     match self.tp {
    //         PatternType::Prefix(ref p) => path.push_str(p),
    //         PatternType::Static(ref p) => path.push_str(p),
    //         PatternType::Dynamic(..) => {
    //             for el in &self.elements {
    //                 match *el {
    //                     PatternElement::Str(ref s) => path.push_str(s),
    //                     PatternElement::Var(_) => {
    //                         if let Some(val) = elements.next() {
    //                             path.push_str(val.as_ref())
    //                         } else {
    //                             return Err(UrlGenerationError::NotEnoughElements);
    //                         }
    //                     }
    //                 }
    //             }
    //         }
    //     };
    //     Ok(())
    // }

    fn parse_param(pattern: &str) -> (PatternElement, String, &str) {
        const DEFAULT_PATTERN: &str = "[^/]+";
        let mut params_nesting = 0usize;
        let close_idx = pattern
            .find(|c| match c {
                '{' => {
                    params_nesting += 1;
                    false
                }
                '}' => {
                    params_nesting -= 1;
                    params_nesting == 0
                }
                _ => false,
            })
            .expect("malformed dynamic segment");
        let (mut param, rem) = pattern.split_at(close_idx + 1);
        param = &param[1..param.len() - 1]; // Remove outer brackets
        let (name, pattern) = match param.find(':') {
            Some(idx) => {
                let (name, pattern) = param.split_at(idx);
                (name, &pattern[1..])
            }
            None => (param, DEFAULT_PATTERN),
        };
        (
            PatternElement::Var(name.to_string()),
            format!(r"(?P<{}>{})", &name, &pattern),
            rem,
        )
    }

    fn parse(
        mut pattern: &str,
        for_prefix: bool,
    ) -> (String, Vec<PatternElement>, bool, usize) {
        if pattern.find('{').is_none() {
            return (
                String::from(pattern),
                vec![PatternElement::Str(String::from(pattern))],
                false,
                pattern.chars().count(),
            );
        };

        let mut elems = Vec::new();
        let mut re = String::from("^");
        let mut dyn_elems = 0;

        while let Some(idx) = pattern.find('{') {
            let (prefix, rem) = pattern.split_at(idx);
            elems.push(PatternElement::Str(String::from(prefix)));
            re.push_str(&escape(prefix));
            let (param_pattern, re_part, rem) = Self::parse_param(rem);
            elems.push(param_pattern);
            re.push_str(&re_part);
            pattern = rem;
            dyn_elems += 1;
        }

        elems.push(PatternElement::Str(String::from(pattern)));
        re.push_str(&escape(pattern));

        if dyn_elems > MAX_DYNAMIC_SEGMENTS {
            panic!(
                "Only {} dynanic segments are allowed, provided: {}",
                MAX_DYNAMIC_SEGMENTS, dyn_elems
            );
        }

        if !for_prefix {
            re.push_str("$");
        }

        (re, elems, true, pattern.chars().count())
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Pattern) -> bool {
        self.pattern == other.pattern
    }
}

impl Eq for Pattern {}

impl Hash for Pattern {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pattern.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_static() {
        let re = Pattern::new("/");
        assert!(re.is_match("/"));
        assert!(!re.is_match("/a"));

        let re = Pattern::new("/name");
        assert!(re.is_match("/name"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name/"));
        assert!(!re.is_match("/name~"));

        let re = Pattern::new("/name/");
        assert!(re.is_match("/name/"));
        assert!(!re.is_match("/name"));
        assert!(!re.is_match("/name/gs"));

        let re = Pattern::new("/user/profile");
        assert!(re.is_match("/user/profile"));
        assert!(!re.is_match("/user/profile/profile"));
    }

    #[test]
    fn test_parse_param() {
        let re = Pattern::new("/user/{id}");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut path = Path::new("/user/profile");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");

        let mut path = Path::new("/user/1245125");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");

        let re = Pattern::new("/v{version}/resource/{id}");
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adahg32");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("version").unwrap(), "151");
        assert_eq!(path.get("id").unwrap(), "adahg32");

        let re = Pattern::new("/{id:[[:digit:]]{6}}");
        assert!(re.is_match("/012345"));
        assert!(!re.is_match("/012"));
        assert!(!re.is_match("/01234567"));
        assert!(!re.is_match("/XXXXXX"));

        let mut path = Path::new("/012345");
        assert!(re.match_path(&mut path));
        assert_eq!(path.get("id").unwrap(), "012345");
    }

    #[test]
    fn test_resource_prefix() {
        let re = Pattern::prefix("/name");
        assert!(re.is_match("/name"));
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/test/test"));
        assert!(re.is_match("/name1"));
        assert!(re.is_match("/name~"));

        let re = Pattern::prefix("/name/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));
    }

    #[test]
    fn test_reousrce_prefix_dynamic() {
        let re = Pattern::prefix("/{name}/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));

        let mut path = Path::new("/test2/");
        assert!(re.match_path(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");

        let mut path = Path::new("/test2/subpath1/subpath2/index.html");
        assert!(re.match_path(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
    }
}
