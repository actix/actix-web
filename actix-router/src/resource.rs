use std::{
    borrow::{Borrow, Cow},
    collections::HashMap,
    hash::{BuildHasher, Hash, Hasher},
    mem,
};

use tracing::error;

use crate::{
    path::PathItem,
    regex_set::{escape, Regex, RegexSet},
    IntoPatterns, Patterns, Resource, ResourcePath,
};

const MAX_DYNAMIC_SEGMENTS: usize = 16;

/// Regex flags to allow '.' in regex to match '\n'
///
/// See the docs under: https://docs.rs/regex/1/regex/#grouping-and-flags
const REGEX_FLAGS: &str = "(?s-m)";

/// Describes the set of paths that match to a resource.
///
/// `ResourceDef`s are effectively a way to transform the a custom resource pattern syntax into
/// suitable regular expressions from which to check matches with paths and capture portions of a
/// matched path into variables. Common cases are on a fast path that avoids going through the
/// regex engine.
///
///
/// # Pattern Format and Matching Behavior
/// Resource pattern is defined as a string of zero or more _segments_ where each segment is
/// preceded by a slash `/`.
///
/// This means that pattern string __must__ either be empty or begin with a slash (`/`). This also
/// implies that a trailing slash in pattern defines an empty segment. For example, the pattern
/// `"/user/"` has two segments: `["user", ""]`
///
/// A key point to understand is that `ResourceDef` matches segments, not strings. Segments are
/// matched individually. For example, the pattern `/user/` is not considered a prefix for the path
/// `/user/123/456`, because the second segment doesn't match: `["user", ""]`
/// vs `["user", "123", "456"]`.
///
/// This definition is consistent with the definition of absolute URL path in
/// [RFC 3986 §3.3](https://datatracker.ietf.org/doc/html/rfc3986#section-3.3)
///
///
/// # Static Resources
/// A static resource is the most basic type of definition. Pass a pattern to [new][Self::new].
/// Conforming paths must match the pattern exactly.
///
/// ## Examples
/// ```
/// # use actix_router::ResourceDef;
/// let resource = ResourceDef::new("/home");
///
/// assert!(resource.is_match("/home"));
///
/// assert!(!resource.is_match("/home/"));
/// assert!(!resource.is_match("/home/new"));
/// assert!(!resource.is_match("/homes"));
/// assert!(!resource.is_match("/search"));
/// ```
///
/// # Dynamic Segments
/// Also known as "path parameters". Resources can define sections of a pattern that be extracted
/// from a conforming path, if it conforms to (one of) the resource pattern(s).
///
/// The marker for a dynamic segment is curly braces wrapping an identifier. For example,
/// `/user/{id}` would match paths like `/user/123` or `/user/james` and be able to extract the user
/// IDs "123" and "james", respectively.
///
/// However, this resource pattern (`/user/{id}`) would, not cover `/user/123/stars` (unless
/// constructed as a prefix; see next section) since the default pattern for segments matches all
/// characters until it finds a `/` character (or the end of the path). Custom segment patterns are
/// covered further down.
///
/// Dynamic segments do not need to be delimited by `/` characters, they can be defined within a
/// path segment. For example, `/rust-is-{opinion}` can match the paths `/rust-is-cool` and
/// `/rust-is-hard`.
///
/// For information on capturing segment values from paths or other custom resource types,
/// see [`capture_match_info`][Self::capture_match_info]
/// and [`capture_match_info_fn`][Self::capture_match_info_fn].
///
/// A resource definition can contain at most 16 dynamic segments.
///
/// ## Examples
/// ```
/// use actix_router::{Path, ResourceDef};
///
/// let resource = ResourceDef::prefix("/user/{id}");
///
/// assert!(resource.is_match("/user/123"));
/// assert!(!resource.is_match("/user"));
/// assert!(!resource.is_match("/user/"));
///
/// let mut path = Path::new("/user/123");
/// resource.capture_match_info(&mut path);
/// assert_eq!(path.get("id").unwrap(), "123");
/// ```
///
/// # Prefix Resources
/// A prefix resource is defined as pattern that can match just the start of a path, up to a
/// segment boundary.
///
/// Prefix patterns with a trailing slash may have an unexpected, though correct, behavior.
/// They define and therefore require an empty segment in order to match. It is easier to understand
/// this behavior after reading the [matching behavior section]. Examples are given below.
///
/// The empty pattern (`""`), as a prefix, matches any path.
///
/// Prefix resources can contain dynamic segments.
///
/// ## Examples
/// ```
/// # use actix_router::ResourceDef;
/// let resource = ResourceDef::prefix("/home");
/// assert!(resource.is_match("/home"));
/// assert!(resource.is_match("/home/new"));
/// assert!(!resource.is_match("/homes"));
///
/// // prefix pattern with a trailing slash
/// let resource = ResourceDef::prefix("/user/{id}/");
/// assert!(resource.is_match("/user/123/"));
/// assert!(resource.is_match("/user/123//stars"));
/// assert!(!resource.is_match("/user/123/stars"));
/// assert!(!resource.is_match("/user/123"));
/// ```
///
/// # Custom Regex Segments
/// Dynamic segments can be customised to only match a specific regular expression. It can be
/// helpful to do this if resource definitions would otherwise conflict and cause one to
/// be inaccessible.
///
/// The regex used when capturing segment values can be specified explicitly using this syntax:
/// `{name:regex}`. For example, `/user/{id:\d+}` will only match paths where the user ID
/// is numeric.
///
/// The regex could potentially match multiple segments. If this is not wanted, then care must be
/// taken to avoid matching a slash `/`. It is guaranteed, however, that the match ends at a
/// segment boundary; the pattern `r"(/|$)` is always appended to the regex.
///
/// By default, dynamic segments use this regex: `[^/]+`. This shows why it is the case, as shown in
/// the earlier section, that segments capture a slice of the path up to the next `/` character.
///
/// Custom regex segments can be used in static and prefix resource definition variants.
///
/// ## Examples
/// ```
/// # use actix_router::ResourceDef;
/// let resource = ResourceDef::new(r"/user/{id:\d+}");
/// assert!(resource.is_match("/user/123"));
/// assert!(resource.is_match("/user/314159"));
/// assert!(!resource.is_match("/user/abc"));
/// ```
///
/// # Tail Segments
/// As a shortcut to defining a custom regex for matching _all_ remaining characters (not just those
/// up until a `/` character), there is a special pattern to match (and capture) the remaining
/// path portion.
///
/// To do this, use the segment pattern: `{name}*`. Since a tail segment also has a name, values are
/// extracted in the same way as non-tail dynamic segments.
///
/// ## Examples
/// ```
/// # use actix_router::{Path, ResourceDef};
/// let resource = ResourceDef::new("/blob/{tail}*");
/// assert!(resource.is_match("/blob/HEAD/Cargo.toml"));
/// assert!(resource.is_match("/blob/HEAD/README.md"));
///
/// let mut path = Path::new("/blob/main/LICENSE");
/// resource.capture_match_info(&mut path);
/// assert_eq!(path.get("tail").unwrap(), "main/LICENSE");
/// ```
///
/// # Multi-Pattern Resources
/// For resources that can map to multiple distinct paths, it may be suitable to use
/// multi-pattern resources by passing an array/vec to [`new`][Self::new]. They will be combined
/// into a regex set which is usually quicker to check matches on than checking each
/// pattern individually.
///
/// Multi-pattern resources can contain dynamic segments just like single pattern ones.
/// However, take care to use consistent and semantically-equivalent segment names; it could affect
/// expectations in the router using these definitions and cause runtime panics.
///
/// ## Examples
/// ```
/// # use actix_router::ResourceDef;
/// let resource = ResourceDef::new(["/home", "/index"]);
/// assert!(resource.is_match("/home"));
/// assert!(resource.is_match("/index"));
/// ```
///
/// # Trailing Slashes
/// It should be noted that this library takes no steps to normalize intra-path or trailing slashes.
/// As such, all resource definitions implicitly expect a pre-processing step to normalize paths if
/// you wish to accommodate "recoverable" path errors. Below are several examples of resource-path
/// pairs that would not be compatible.
///
/// ## Examples
/// ```
/// # use actix_router::ResourceDef;
/// assert!(!ResourceDef::new("/root").is_match("/root/"));
/// assert!(!ResourceDef::new("/root/").is_match("/root"));
/// assert!(!ResourceDef::prefix("/root/").is_match("/root"));
/// ```
///
/// [matching behavior section]: #pattern-format-and-matching-behavior
#[derive(Clone, Debug)]
pub struct ResourceDef {
    id: u16,

    /// Optional name of resource.
    name: Option<String>,

    /// Pattern that generated the resource definition.
    patterns: Patterns,

    is_prefix: bool,

    /// Pattern type.
    pat_type: PatternType,

    /// List of segments that compose the pattern, in order.
    segments: Vec<PatternSegment>,
}

#[derive(Debug, Clone, PartialEq)]
enum PatternSegment {
    /// Literal slice of pattern.
    Const(String),

    /// Name of dynamic segment.
    Var(String),
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum PatternType {
    /// Single constant/literal segment.
    Static(String),

    /// Single regular expression and list of dynamic segment names.
    Dynamic(Regex, Vec<&'static str>),

    /// Regular expression set and list of component expressions plus dynamic segment names.
    DynamicSet(RegexSet, Vec<(Regex, Vec<&'static str>)>),
}

impl ResourceDef {
    /// Constructs a new resource definition from patterns.
    ///
    /// Multi-pattern resources can be constructed by providing a slice (or vec) of patterns.
    ///
    /// # Panics
    /// Panics if any path patterns are malformed.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// let resource = ResourceDef::new("/user/{id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(!resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("user/1234"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// let resource = ResourceDef::new(["/profile", "/user/{id}"]);
    /// assert!(resource.is_match("/profile"));
    /// assert!(resource.is_match("/user/123"));
    /// assert!(!resource.is_match("user/123"));
    /// assert!(!resource.is_match("/foo"));
    /// ```
    pub fn new<T: IntoPatterns>(paths: T) -> Self {
        Self::construct(paths, false)
    }

    /// Constructs a new resource definition using a pattern that performs prefix matching.
    ///
    /// More specifically, the regular expressions generated for matching are different when using
    /// this method vs using `new`; they will not be appended with the `$` meta-character that
    /// matches the end of an input.
    ///
    /// Although it will compile and run correctly, it is meaningless to construct a prefix
    /// resource definition with a tail segment; use [`new`][Self::new] in this case.
    ///
    /// # Panics
    /// Panics if path pattern is malformed.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// let resource = ResourceDef::prefix("/user/{id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(resource.is_match("/user/123/stars"));
    /// assert!(!resource.is_match("user/123"));
    /// assert!(!resource.is_match("user/123/stars"));
    /// assert!(!resource.is_match("/foo"));
    /// ```
    pub fn prefix<T: IntoPatterns>(paths: T) -> Self {
        ResourceDef::construct(paths, true)
    }

    /// Constructs a new resource definition using a string pattern that performs prefix matching,
    /// ensuring a leading `/` if pattern is not empty.
    ///
    /// # Panics
    /// Panics if path pattern is malformed.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// let resource = ResourceDef::root_prefix("user/{id}");
    ///
    /// assert_eq!(&resource, &ResourceDef::prefix("/user/{id}"));
    /// assert_eq!(&resource, &ResourceDef::root_prefix("/user/{id}"));
    /// assert_ne!(&resource, &ResourceDef::new("user/{id}"));
    /// assert_ne!(&resource, &ResourceDef::new("/user/{id}"));
    ///
    /// assert!(resource.is_match("/user/123"));
    /// assert!(!resource.is_match("user/123"));
    /// ```
    pub fn root_prefix(path: &str) -> Self {
        ResourceDef::prefix(insert_slash(path).into_owned())
    }

    /// Returns a numeric resource ID.
    ///
    /// If not explicitly set using [`set_id`][Self::set_id], this will return `0`.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// assert_eq!(resource.id(), 0);
    ///
    /// resource.set_id(42);
    /// assert_eq!(resource.id(), 42);
    /// ```
    pub fn id(&self) -> u16 {
        self.id
    }

    /// Set numeric resource ID.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// resource.set_id(42);
    /// assert_eq!(resource.id(), 42);
    /// ```
    pub fn set_id(&mut self, id: u16) {
        self.id = id;
    }

    /// Returns resource definition name, if set.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// assert!(resource.name().is_none());
    ///
    /// resource.set_name("root");
    /// assert_eq!(resource.name().unwrap(), "root");
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Assigns a new name to the resource.
    ///
    /// # Panics
    /// Panics if `name` is an empty string.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// resource.set_name("root");
    /// assert_eq!(resource.name().unwrap(), "root");
    /// ```
    pub fn set_name(&mut self, name: impl Into<String>) {
        let name = name.into();

        assert!(!name.is_empty(), "resource name should not be empty");

        self.name = Some(name)
    }

    /// Returns `true` if pattern type is prefix.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// assert!(ResourceDef::prefix("/user").is_prefix());
    /// assert!(!ResourceDef::new("/user").is_prefix());
    /// ```
    pub fn is_prefix(&self) -> bool {
        self.is_prefix
    }

    /// Returns the pattern string that generated the resource definition.
    ///
    /// If definition is constructed with multiple patterns, the first pattern is returned. To get
    /// all patterns, use [`patterns_iter`][Self::pattern_iter]. If resource has 0 patterns,
    /// returns `None`.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/user/{id}");
    /// assert_eq!(resource.pattern().unwrap(), "/user/{id}");
    ///
    /// let mut resource = ResourceDef::new(["/profile", "/user/{id}"]);
    /// assert_eq!(resource.pattern(), Some("/profile"));
    pub fn pattern(&self) -> Option<&str> {
        match &self.patterns {
            Patterns::Single(pattern) => Some(pattern.as_str()),
            Patterns::List(patterns) => patterns.first().map(AsRef::as_ref),
        }
    }

    /// Returns iterator of pattern strings that generated the resource definition.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut resource = ResourceDef::new("/root");
    /// let mut iter = resource.pattern_iter();
    /// assert_eq!(iter.next().unwrap(), "/root");
    /// assert!(iter.next().is_none());
    ///
    /// let mut resource = ResourceDef::new(["/root", "/backup"]);
    /// let mut iter = resource.pattern_iter();
    /// assert_eq!(iter.next().unwrap(), "/root");
    /// assert_eq!(iter.next().unwrap(), "/backup");
    /// assert!(iter.next().is_none());
    pub fn pattern_iter(&self) -> impl Iterator<Item = &str> {
        struct PatternIter<'a> {
            patterns: &'a Patterns,
            list_idx: usize,
            done: bool,
        }

        impl<'a> Iterator for PatternIter<'a> {
            type Item = &'a str;

            fn next(&mut self) -> Option<Self::Item> {
                match &self.patterns {
                    Patterns::Single(pattern) => {
                        if self.done {
                            return None;
                        }

                        self.done = true;
                        Some(pattern.as_str())
                    }
                    Patterns::List(patterns) if patterns.is_empty() => None,
                    Patterns::List(patterns) => match patterns.get(self.list_idx) {
                        Some(pattern) => {
                            self.list_idx += 1;
                            Some(pattern.as_str())
                        }
                        None => {
                            // fast path future call
                            self.done = true;
                            None
                        }
                    },
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                match &self.patterns {
                    Patterns::Single(_) => (1, Some(1)),
                    Patterns::List(patterns) => (patterns.len(), Some(patterns.len())),
                }
            }
        }

        PatternIter {
            patterns: &self.patterns,
            list_idx: 0,
            done: false,
        }
    }

    /// Joins two resources.
    ///
    /// Resulting resource is prefix if `other` is prefix.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let joined = ResourceDef::prefix("/root").join(&ResourceDef::prefix("/seg"));
    /// assert_eq!(joined, ResourceDef::prefix("/root/seg"));
    /// ```
    pub fn join(&self, other: &ResourceDef) -> ResourceDef {
        let patterns = self
            .pattern_iter()
            .flat_map(move |this| other.pattern_iter().map(move |other| (this, other)))
            .map(|(this, other)| {
                let mut pattern = String::with_capacity(this.len() + other.len());
                pattern.push_str(this);
                pattern.push_str(other);
                pattern
            })
            .collect::<Vec<_>>();

        match patterns.len() {
            1 => ResourceDef::construct(&patterns[0], other.is_prefix()),
            _ => ResourceDef::construct(patterns, other.is_prefix()),
        }
    }

    /// Returns `true` if `path` matches this resource.
    ///
    /// The behavior of this method depends on how the `ResourceDef` was constructed. For example,
    /// static resources will not be able to match as many paths as dynamic and prefix resources.
    /// See [`ResourceDef`] struct docs for details on resource definition types.
    ///
    /// This method will always agree with [`find_match`][Self::find_match] on whether the path
    /// matches or not.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// // static resource
    /// let resource = ResourceDef::new("/user");
    /// assert!(resource.is_match("/user"));
    /// assert!(!resource.is_match("/users"));
    /// assert!(!resource.is_match("/user/123"));
    /// assert!(!resource.is_match("/foo"));
    ///
    /// // dynamic resource
    /// let resource = ResourceDef::new("/user/{user_id}");
    /// assert!(resource.is_match("/user/123"));
    /// assert!(!resource.is_match("/user/123/stars"));
    ///
    /// // prefix resource
    /// let resource = ResourceDef::prefix("/root");
    /// assert!(resource.is_match("/root"));
    /// assert!(resource.is_match("/root/leaf"));
    /// assert!(!resource.is_match("/roots"));
    ///
    /// // more examples are shown in the `ResourceDef` struct docs
    /// ```
    #[inline]
    pub fn is_match(&self, path: &str) -> bool {
        // this function could be expressed as:
        // `self.find_match(path).is_some()`
        // but this skips some checks and uses potentially faster regex methods

        match &self.pat_type {
            PatternType::Static(pattern) => self.static_match(pattern, path).is_some(),
            PatternType::Dynamic(re, _) => re.is_match(path),
            PatternType::DynamicSet(re, _) => re.is_match(path),
        }
    }

    /// Tries to match `path` to this resource, returning the position in the path where the
    /// match ends.
    ///
    /// This method will always agree with [`is_match`][Self::is_match] on whether the path matches
    /// or not.
    ///
    /// # Examples
    /// ```
    /// use actix_router::ResourceDef;
    ///
    /// // static resource
    /// let resource = ResourceDef::new("/user");
    /// assert_eq!(resource.find_match("/user"), Some(5));
    /// assert!(resource.find_match("/user/").is_none());
    /// assert!(resource.find_match("/user/123").is_none());
    /// assert!(resource.find_match("/foo").is_none());
    ///
    /// // constant prefix resource
    /// let resource = ResourceDef::prefix("/user");
    /// assert_eq!(resource.find_match("/user"), Some(5));
    /// assert_eq!(resource.find_match("/user/"), Some(5));
    /// assert_eq!(resource.find_match("/user/123"), Some(5));
    ///
    /// // dynamic prefix resource
    /// let resource = ResourceDef::prefix("/user/{id}");
    /// assert_eq!(resource.find_match("/user/123"), Some(9));
    /// assert_eq!(resource.find_match("/user/1234/"), Some(10));
    /// assert_eq!(resource.find_match("/user/12345/stars"), Some(11));
    /// assert!(resource.find_match("/user/").is_none());
    ///
    /// // multi-pattern resource
    /// let resource = ResourceDef::new(["/user/{id}", "/profile/{id}"]);
    /// assert_eq!(resource.find_match("/user/123"), Some(9));
    /// assert_eq!(resource.find_match("/profile/1234"), Some(13));
    /// ```
    pub fn find_match(&self, path: &str) -> Option<usize> {
        match &self.pat_type {
            PatternType::Static(pattern) => self.static_match(pattern, path),

            PatternType::Dynamic(re, _) => Some(re.captures(path)?[1].len()),

            PatternType::DynamicSet(re, params) => {
                let idx = re.first_match_idx(path)?;
                let (ref pattern, _) = params[idx];
                Some(pattern.captures(path)?[1].len())
            }
        }
    }

    /// Collects dynamic segment values into `resource`.
    ///
    /// Returns `true` if `path` matches this resource.
    ///
    /// # Examples
    /// ```
    /// use actix_router::{Path, ResourceDef};
    ///
    /// let resource = ResourceDef::prefix("/user/{id}");
    /// let mut path = Path::new("/user/123/stars");
    /// assert!(resource.capture_match_info(&mut path));
    /// assert_eq!(path.get("id").unwrap(), "123");
    /// assert_eq!(path.unprocessed(), "/stars");
    ///
    /// let resource = ResourceDef::new("/blob/{path}*");
    /// let mut path = Path::new("/blob/HEAD/Cargo.toml");
    /// assert!(resource.capture_match_info(&mut path));
    /// assert_eq!(path.get("path").unwrap(), "HEAD/Cargo.toml");
    /// assert_eq!(path.unprocessed(), "");
    /// ```
    pub fn capture_match_info<R: Resource>(&self, resource: &mut R) -> bool {
        self.capture_match_info_fn(resource, |_| true)
    }

    /// Collects dynamic segment values into `resource` after matching paths and executing
    /// check function.
    ///
    /// The check function is given a reference to the passed resource and optional arbitrary data.
    /// This is useful if you want to conditionally match on some non-path related aspect of the
    /// resource type.
    ///
    /// Returns `true` if resource path matches this resource definition _and_ satisfies the
    /// given check function.
    ///
    /// # Examples
    /// ```
    /// use actix_router::{Path, ResourceDef};
    ///
    /// fn try_match(resource: &ResourceDef, path: &mut Path<&str>) -> bool {
    ///     let admin_allowed = std::env::var("ADMIN_ALLOWED").is_ok();
    ///
    ///     resource.capture_match_info_fn(
    ///         path,
    ///         // when env var is not set, reject when path contains "admin"
    ///         |path| !(!admin_allowed && path.as_str().contains("admin")),
    ///     )
    /// }
    ///
    /// let resource = ResourceDef::prefix("/user/{id}");
    ///
    /// // path matches; segment values are collected into path
    /// let mut path = Path::new("/user/james/stars");
    /// assert!(try_match(&resource, &mut path));
    /// assert_eq!(path.get("id").unwrap(), "james");
    /// assert_eq!(path.unprocessed(), "/stars");
    ///
    /// // path matches but fails check function; no segments are collected
    /// let mut path = Path::new("/user/admin/stars");
    /// assert!(!try_match(&resource, &mut path));
    /// assert_eq!(path.unprocessed(), "/user/admin/stars");
    /// ```
    pub fn capture_match_info_fn<R, F>(&self, resource: &mut R, check_fn: F) -> bool
    where
        R: Resource,
        F: FnOnce(&R) -> bool,
    {
        let mut segments = <[PathItem; MAX_DYNAMIC_SEGMENTS]>::default();
        let path = resource.resource_path();
        let path_str = path.unprocessed();

        let (matched_len, matched_vars) = match &self.pat_type {
            PatternType::Static(pattern) => match self.static_match(pattern, path_str) {
                Some(len) => (len, None),
                None => return false,
            },

            PatternType::Dynamic(re, names) => {
                let captures = match re.captures(path.unprocessed()) {
                    Some(captures) => captures,
                    _ => return false,
                };

                for (no, name) in names.iter().enumerate() {
                    if let Some(m) = captures.name(name) {
                        segments[no] = PathItem::Segment(m.start() as u16, m.end() as u16);
                    } else {
                        error!("Dynamic path match but not all segments found: {}", name);
                        return false;
                    }
                }

                (captures[1].len(), Some(names))
            }

            PatternType::DynamicSet(re, params) => {
                let path = path.unprocessed();
                let (pattern, names) = match re.first_match_idx(path) {
                    Some(idx) => &params[idx],
                    _ => return false,
                };

                let captures = match pattern.captures(path.path()) {
                    Some(captures) => captures,
                    _ => return false,
                };

                for (no, name) in names.iter().enumerate() {
                    if let Some(m) = captures.name(name) {
                        segments[no] = PathItem::Segment(m.start() as u16, m.end() as u16);
                    } else {
                        error!("Dynamic path match but not all segments found: {}", name);
                        return false;
                    }
                }

                (captures[1].len(), Some(names))
            }
        };

        if !check_fn(resource) {
            return false;
        }

        // Modify `path` to skip matched part and store matched segments
        let path = resource.resource_path();

        if let Some(vars) = matched_vars {
            for i in 0..vars.len() {
                path.add(vars[i], mem::take(&mut segments[i]));
            }
        }

        path.skip(matched_len as u16);

        true
    }

    /// Assembles resource path using a closure that maps variable segment names to values.
    fn build_resource_path<F, I>(&self, path: &mut String, mut vars: F) -> bool
    where
        F: FnMut(&str) -> Option<I>,
        I: AsRef<str>,
    {
        for segment in &self.segments {
            match segment {
                PatternSegment::Const(val) => path.push_str(val),
                PatternSegment::Var(name) => match vars(name) {
                    Some(val) => path.push_str(val.as_ref()),
                    _ => return false,
                },
            }
        }

        true
    }

    /// Assembles full resource path from iterator of dynamic segment values.
    ///
    /// Returns `true` on success.
    ///
    /// For multi-pattern resources, the first pattern is used under the assumption that it would be
    /// equivalent to any other choice.
    ///
    /// # Examples
    /// ```
    /// # use actix_router::ResourceDef;
    /// let mut s = String::new();
    /// let resource = ResourceDef::new("/user/{id}/post/{title}");
    ///
    /// assert!(resource.resource_path_from_iter(&mut s, &["123", "my-post"]));
    /// assert_eq!(s, "/user/123/post/my-post");
    /// ```
    pub fn resource_path_from_iter<I>(&self, path: &mut String, values: I) -> bool
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut iter = values.into_iter();
        self.build_resource_path(path, |_| iter.next())
    }

    /// Assembles resource path from map of dynamic segment values.
    ///
    /// Returns `true` on success.
    ///
    /// For multi-pattern resources, the first pattern is used under the assumption that it would be
    /// equivalent to any other choice.
    ///
    /// # Examples
    /// ```
    /// # use std::collections::HashMap;
    /// # use actix_router::ResourceDef;
    /// let mut s = String::new();
    /// let resource = ResourceDef::new("/user/{id}/post/{title}");
    ///
    /// let mut map = HashMap::new();
    /// map.insert("id", "123");
    /// map.insert("title", "my-post");
    ///
    /// assert!(resource.resource_path_from_map(&mut s, &map));
    /// assert_eq!(s, "/user/123/post/my-post");
    /// ```
    pub fn resource_path_from_map<K, V, S>(
        &self,
        path: &mut String,
        values: &HashMap<K, V, S>,
    ) -> bool
    where
        K: Borrow<str> + Eq + Hash,
        V: AsRef<str>,
        S: BuildHasher,
    {
        self.build_resource_path(path, |name| values.get(name))
    }

    /// Returns true if `prefix` acts as a proper prefix (i.e., separated by a slash) in `path`.
    fn static_match(&self, pattern: &str, path: &str) -> Option<usize> {
        let rem = path.strip_prefix(pattern)?;

        match self.is_prefix {
            // resource is not a prefix so an exact match is needed
            false if rem.is_empty() => Some(pattern.len()),

            // resource is a prefix so rem should start with a path delimiter
            true if rem.is_empty() || rem.starts_with('/') => Some(pattern.len()),

            // otherwise, no match
            _ => None,
        }
    }

    fn construct<T: IntoPatterns>(paths: T, is_prefix: bool) -> Self {
        let patterns = paths.patterns();

        let (pat_type, segments) = match &patterns {
            Patterns::Single(pattern) => ResourceDef::parse(pattern, is_prefix, false),

            // since zero length pattern sets are possible
            // just return a useless `ResourceDef`
            Patterns::List(patterns) if patterns.is_empty() => (
                PatternType::DynamicSet(RegexSet::empty(), Vec::new()),
                Vec::new(),
            ),

            Patterns::List(patterns) => {
                let mut re_set = Vec::with_capacity(patterns.len());
                let mut pattern_data = Vec::new();
                let mut segments = None;

                for pattern in patterns {
                    match ResourceDef::parse(pattern, is_prefix, true) {
                        (PatternType::Dynamic(re, names), segs) => {
                            re_set.push(re.as_str().to_owned());
                            pattern_data.push((re, names));
                            segments.get_or_insert(segs);
                        }
                        _ => unreachable!(),
                    }
                }

                let pattern_re_set = RegexSet::new(re_set);
                let segments = segments.unwrap_or_default();

                (
                    PatternType::DynamicSet(pattern_re_set, pattern_data),
                    segments,
                )
            }
        };

        ResourceDef {
            id: 0,
            name: None,
            patterns,
            is_prefix,
            pat_type,
            segments,
        }
    }

    /// Parses a dynamic segment definition from a pattern.
    ///
    /// The returned tuple includes:
    /// - the segment descriptor, either `Var` or `Tail`
    /// - the segment's regex to check values against
    /// - the remaining, unprocessed string slice
    /// - whether the parsed parameter represents a tail pattern
    ///
    /// # Panics
    /// Panics if given patterns does not contain a dynamic segment.
    fn parse_param(pattern: &str) -> (PatternSegment, String, &str, bool) {
        const DEFAULT_PATTERN: &str = "[^/]+";
        const DEFAULT_PATTERN_TAIL: &str = ".*";

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
            .unwrap_or_else(|| {
                panic!(
                    r#"pattern "{}" contains malformed dynamic segment"#,
                    pattern
                )
            });

        let (mut param, mut unprocessed) = pattern.split_at(close_idx + 1);

        // remove outer curly brackets
        param = &param[1..param.len() - 1];

        let tail = unprocessed == "*";

        let (name, pattern) = match param.find(':') {
            Some(idx) => {
                assert!(!tail, "custom regex is not supported for tail match");

                let (name, pattern) = param.split_at(idx);
                (name, &pattern[1..])
            }
            None => (
                param,
                if tail {
                    unprocessed = &unprocessed[1..];
                    DEFAULT_PATTERN_TAIL
                } else {
                    DEFAULT_PATTERN
                },
            ),
        };

        let segment = PatternSegment::Var(name.to_string());
        let regex = format!(r"(?P<{}>{})", &name, &pattern);

        (segment, regex, unprocessed, tail)
    }

    /// Parse `pattern` using `is_prefix` and `force_dynamic` flags.
    ///
    /// Parameters:
    /// - `is_prefix`: Use `true` if `pattern` should be treated as a prefix; i.e., a conforming
    ///   path will be a match even if it has parts remaining to process
    /// - `force_dynamic`: Use `true` to disallow the return of static and prefix segments.
    ///
    /// The returned tuple includes:
    /// - the pattern type detected, either `Static`, `Prefix`, or `Dynamic`
    /// - a list of segment descriptors from the pattern
    fn parse(
        pattern: &str,
        is_prefix: bool,
        force_dynamic: bool,
    ) -> (PatternType, Vec<PatternSegment>) {
        if !force_dynamic && pattern.find('{').is_none() && !pattern.ends_with('*') {
            // pattern is static
            return (
                PatternType::Static(pattern.to_owned()),
                vec![PatternSegment::Const(pattern.to_owned())],
            );
        }

        let mut unprocessed = pattern;
        let mut segments = Vec::new();
        let mut re = format!("{}^", REGEX_FLAGS);
        let mut dyn_segment_count = 0;
        let mut has_tail_segment = false;

        while let Some(idx) = unprocessed.find('{') {
            let (prefix, rem) = unprocessed.split_at(idx);

            segments.push(PatternSegment::Const(prefix.to_owned()));
            re.push_str(&escape(prefix));

            let (param_pattern, re_part, rem, tail) = Self::parse_param(rem);

            if tail {
                has_tail_segment = true;
            }

            segments.push(param_pattern);
            re.push_str(&re_part);

            unprocessed = rem;
            dyn_segment_count += 1;
        }

        if is_prefix && has_tail_segment {
            // tail segments in prefixes have no defined semantics

            #[cfg(not(test))]
            tracing::warn!(
                "Prefix resources should not have tail segments. \
                Use `ResourceDef::new` constructor. \
                This may become a panic in the future."
            );

            // panic in tests to make this case detectable
            #[cfg(test)]
            panic!("prefix resource definitions should not have tail segments");
        }

        if unprocessed.ends_with('*') {
            // unnamed tail segment

            #[cfg(not(test))]
            tracing::warn!(
                "Tail segments must have names. \
                Consider `.../{{tail}}*`. \
                This may become a panic in the future."
            );

            // panic in tests to make this case detectable
            #[cfg(test)]
            panic!("tail segments must have names");
        } else if !has_tail_segment && !unprocessed.is_empty() {
            // prevent `Const("")` element from being added after last dynamic segment

            segments.push(PatternSegment::Const(unprocessed.to_owned()));
            re.push_str(&escape(unprocessed));
        }

        assert!(
            dyn_segment_count <= MAX_DYNAMIC_SEGMENTS,
            "Only {} dynamic segments are allowed, provided: {}",
            MAX_DYNAMIC_SEGMENTS,
            dyn_segment_count
        );

        // Store the pattern in capture group #1 to have context info outside it
        let mut re = format!("({})", re);

        // Ensure the match ends at a segment boundary
        if !has_tail_segment {
            if is_prefix {
                re.push_str(r"(/|$)");
            } else {
                re.push('$');
            }
        }

        let re = match Regex::new(&re) {
            Ok(re) => re,
            Err(err) => panic!("Wrong path pattern: \"{}\" {}", pattern, err),
        };

        // `Bok::leak(Box::new(name))` is an intentional memory leak. In typical applications the
        // routing table is only constructed once (per worker) so leak is bounded. If you are
        // constructing `ResourceDef`s more than once in your application's lifecycle you would
        // expect a linear increase in leaked memory over time.
        let names = re
            .capture_names()
            .filter_map(|name| name.map(|name| Box::leak(Box::new(name.to_owned())).as_str()))
            .collect();

        (PatternType::Dynamic(re, names), segments)
    }
}

impl Eq for ResourceDef {}

impl PartialEq for ResourceDef {
    fn eq(&self, other: &ResourceDef) -> bool {
        self.patterns == other.patterns && self.is_prefix == other.is_prefix
    }
}

impl Hash for ResourceDef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.patterns.hash(state);
    }
}

impl<'a> From<&'a str> for ResourceDef {
    fn from(path: &'a str) -> ResourceDef {
        ResourceDef::new(path)
    }
}

impl From<String> for ResourceDef {
    fn from(path: String) -> ResourceDef {
        ResourceDef::new(path)
    }
}

pub(crate) fn insert_slash(path: &str) -> Cow<'_, str> {
    if !path.is_empty() && !path.starts_with('/') {
        let mut new_path = String::with_capacity(path.len() + 1);
        new_path.push('/');
        new_path.push_str(path);
        Cow::Owned(new_path)
    } else {
        Cow::Borrowed(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Path;

    #[test]
    fn equivalence() {
        assert_eq!(
            ResourceDef::root_prefix("/root"),
            ResourceDef::prefix("/root")
        );
        assert_eq!(
            ResourceDef::root_prefix("root"),
            ResourceDef::prefix("/root")
        );
        assert_eq!(
            ResourceDef::root_prefix("/{id}"),
            ResourceDef::prefix("/{id}")
        );
        assert_eq!(
            ResourceDef::root_prefix("{id}"),
            ResourceDef::prefix("/{id}")
        );

        assert_eq!(ResourceDef::new("/"), ResourceDef::new(["/"]));
        assert_eq!(ResourceDef::new("/"), ResourceDef::new(vec!["/"]));

        assert_ne!(ResourceDef::new(""), ResourceDef::prefix(""));
        assert_ne!(ResourceDef::new("/"), ResourceDef::prefix("/"));
        assert_ne!(ResourceDef::new("/{id}"), ResourceDef::prefix("/{id}"));
    }

    #[test]
    fn parse_static() {
        let re = ResourceDef::new("");

        assert!(!re.is_prefix());

        assert!(re.is_match(""));
        assert!(!re.is_match("/"));
        assert_eq!(re.find_match(""), Some(0));
        assert_eq!(re.find_match("/"), None);

        let re = ResourceDef::new("/");
        assert!(re.is_match("/"));
        assert!(!re.is_match(""));
        assert!(!re.is_match("/foo"));

        let re = ResourceDef::new("/name");
        assert!(re.is_match("/name"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name/"));
        assert!(!re.is_match("/name~"));

        let mut path = Path::new("/name");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.unprocessed(), "");

        assert_eq!(re.find_match("/name"), Some(5));
        assert_eq!(re.find_match("/name1"), None);
        assert_eq!(re.find_match("/name/"), None);
        assert_eq!(re.find_match("/name~"), None);

        let re = ResourceDef::new("/name/");
        assert!(re.is_match("/name/"));
        assert!(!re.is_match("/name"));
        assert!(!re.is_match("/name/gs"));

        let re = ResourceDef::new("/user/profile");
        assert!(re.is_match("/user/profile"));
        assert!(!re.is_match("/user/profile/profile"));

        let mut path = Path::new("/user/profile");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.unprocessed(), "");
    }

    #[test]
    fn parse_param() {
        let re = ResourceDef::new("/user/{id}");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut path = Path::new("/user/profile");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/user/1245125");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");
        assert_eq!(path.unprocessed(), "");

        let re = ResourceDef::new("/v{version}/resource/{id}");
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adage32");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("version").unwrap(), "151");
        assert_eq!(path.get("id").unwrap(), "adage32");
        assert_eq!(path.unprocessed(), "");

        let re = ResourceDef::new("/{id:[[:digit:]]{6}}");
        assert!(re.is_match("/012345"));
        assert!(!re.is_match("/012"));
        assert!(!re.is_match("/01234567"));
        assert!(!re.is_match("/XXXXXX"));

        let mut path = Path::new("/012345");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "012345");
        assert_eq!(path.unprocessed(), "");
    }

    #[allow(clippy::cognitive_complexity)]
    #[test]
    fn dynamic_set() {
        let re = ResourceDef::new(vec![
            "/user/{id}",
            "/v{version}/resource/{id}",
            "/{id:[[:digit:]]{6}}",
            "/static",
        ]);
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut path = Path::new("/user/profile");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/user/1245125");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "1245125");
        assert_eq!(path.unprocessed(), "");

        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut path = Path::new("/v151/resource/adage32");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("version").unwrap(), "151");
        assert_eq!(path.get("id").unwrap(), "adage32");

        assert!(re.is_match("/012345"));
        assert!(!re.is_match("/012"));
        assert!(!re.is_match("/01234567"));
        assert!(!re.is_match("/XXXXXX"));

        assert!(re.is_match("/static"));
        assert!(!re.is_match("/a/static"));
        assert!(!re.is_match("/static/a"));

        let mut path = Path::new("/012345");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "012345");

        let re = ResourceDef::new([
            "/user/{id}",
            "/v{version}/resource/{id}",
            "/{id:[[:digit:]]{6}}",
        ]);
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let re = ResourceDef::new([
            "/user/{id}".to_string(),
            "/v{version}/resource/{id}".to_string(),
            "/{id:[[:digit:]]{6}}".to_string(),
        ]);
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));
    }

    #[test]
    fn dynamic_set_prefix() {
        let re = ResourceDef::prefix(vec!["/u/{id}", "/{id:[[:digit:]]{3}}"]);

        assert_eq!(re.find_match("/u/abc"), Some(6));
        assert_eq!(re.find_match("/u/abc/123"), Some(6));
        assert_eq!(re.find_match("/s/user/profile"), None);

        assert_eq!(re.find_match("/123"), Some(4));
        assert_eq!(re.find_match("/123/456"), Some(4));
        assert_eq!(re.find_match("/12345"), None);

        let mut path = Path::new("/151/res");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "151");
        assert_eq!(path.unprocessed(), "/res");
    }

    #[test]
    fn parse_tail() {
        let re = ResourceDef::new("/user/-{id}*");

        let mut path = Path::new("/user/-profile");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "profile");

        let mut path = Path::new("/user/-2345");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");

        let mut path = Path::new("/user/-2345/");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345/");

        let mut path = Path::new("/user/-2345/sdg");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345/sdg");
    }

    #[test]
    fn static_tail() {
        let re = ResourceDef::new("/user{tail}*");
        assert!(re.is_match("/users"));
        assert!(re.is_match("/user-foo"));
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(re.is_match("/user/2345/"));
        assert!(re.is_match("/user/2345/sdg"));
        assert!(!re.is_match("/foo/profile"));

        let re = ResourceDef::new("/user/{tail}*");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(re.is_match("/user/2345/"));
        assert!(re.is_match("/user/2345/sdg"));
        assert!(!re.is_match("/foo/profile"));
    }

    #[test]
    fn dynamic_tail() {
        let re = ResourceDef::new("/user/{id}/{tail}*");
        assert!(!re.is_match("/user/2345"));
        let mut path = Path::new("/user/2345/sdg");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");
        assert_eq!(path.get("tail").unwrap(), "sdg");
        assert_eq!(path.unprocessed(), "");
    }

    #[test]
    fn newline_patterns_and_paths() {
        let re = ResourceDef::new("/user/a\nb");
        assert!(re.is_match("/user/a\nb"));
        assert!(!re.is_match("/user/a\nb/profile"));

        let re = ResourceDef::new("/a{x}b/test/a{y}b");
        let mut path = Path::new("/a\nb/test/a\nb");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("x").unwrap(), "\n");
        assert_eq!(path.get("y").unwrap(), "\n");

        let re = ResourceDef::new("/user/{tail}*");
        assert!(re.is_match("/user/a\nb/"));

        let re = ResourceDef::new("/user/{id}*");
        let mut path = Path::new("/user/a\nb/a\nb");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "a\nb/a\nb");

        let re = ResourceDef::new("/user/{id:.*}");
        let mut path = Path::new("/user/a\nb/a\nb");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "a\nb/a\nb");
    }

    #[cfg(feature = "http")]
    #[test]
    fn parse_urlencoded_param() {
        let re = ResourceDef::new("/user/{id}/test");

        let mut path = Path::new("/user/2345/test");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "2345");

        let mut path = Path::new("/user/qwe%25/test");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "qwe%25");

        let uri = http::Uri::try_from("/user/qwe%25/test").unwrap();
        let mut path = Path::new(uri);
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.get("id").unwrap(), "qwe%25");
    }

    #[test]
    fn prefix_static() {
        let re = ResourceDef::prefix("/name");

        assert!(re.is_prefix());

        assert!(re.is_match("/name"));
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/test/test"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name~"));

        let mut path = Path::new("/name");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.unprocessed(), "");

        let mut path = Path::new("/name/test");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.unprocessed(), "/test");

        assert_eq!(re.find_match("/name"), Some(5));
        assert_eq!(re.find_match("/name/"), Some(5));
        assert_eq!(re.find_match("/name/test/test"), Some(5));
        assert_eq!(re.find_match("/name1"), None);
        assert_eq!(re.find_match("/name~"), None);

        let re = ResourceDef::prefix("/name/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name//gs"));
        assert!(!re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));

        let mut path = Path::new("/name/gs");
        assert!(!re.capture_match_info(&mut path));

        let mut path = Path::new("/name//gs");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(path.unprocessed(), "/gs");

        let re = ResourceDef::root_prefix("name/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name//gs"));
        assert!(!re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));

        let mut path = Path::new("/name/gs");
        assert!(!re.capture_match_info(&mut path));
    }

    #[test]
    fn prefix_dynamic() {
        let re = ResourceDef::prefix("/{name}");

        assert!(re.is_prefix());

        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(re.is_match("/name"));

        assert_eq!(re.find_match("/name/"), Some(5));
        assert_eq!(re.find_match("/name/gs"), Some(5));
        assert_eq!(re.find_match("/name"), Some(5));
        assert_eq!(re.find_match(""), None);

        let mut path = Path::new("/test2/");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
        assert_eq!(path.unprocessed(), "/");

        let mut path = Path::new("/test2/subpath1/subpath2/index.html");
        assert!(re.capture_match_info(&mut path));
        assert_eq!(&path["name"], "test2");
        assert_eq!(&path[0], "test2");
        assert_eq!(path.unprocessed(), "/subpath1/subpath2/index.html");

        let resource = ResourceDef::prefix("/user");
        // input string shorter than prefix
        assert!(resource.find_match("/foo").is_none());
    }

    #[test]
    fn prefix_empty() {
        let re = ResourceDef::prefix("");

        assert!(re.is_prefix());

        assert!(re.is_match(""));
        assert!(re.is_match("/"));
        assert!(re.is_match("/name/test/test"));
    }

    #[test]
    fn build_path_list() {
        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/test");
        assert!(resource.resource_path_from_iter(&mut s, &mut ["user1"].iter()));
        assert_eq!(s, "/user/user1/test");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}/test");
        assert!(resource.resource_path_from_iter(&mut s, &mut ["item", "item2"].iter()));
        assert_eq!(s, "/user/item/item2/test");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}");
        assert!(resource.resource_path_from_iter(&mut s, &mut ["item", "item2"].iter()));
        assert_eq!(s, "/user/item/item2");

        let mut s = String::new();
        let resource = ResourceDef::new("/user/{item1}/{item2}/");
        assert!(resource.resource_path_from_iter(&mut s, &mut ["item", "item2"].iter()));
        assert_eq!(s, "/user/item/item2/");

        let mut s = String::new();
        assert!(!resource.resource_path_from_iter(&mut s, &mut ["item"].iter()));

        let mut s = String::new();
        assert!(resource.resource_path_from_iter(&mut s, &mut ["item", "item2"].iter()));
        assert_eq!(s, "/user/item/item2/");
        assert!(!resource.resource_path_from_iter(&mut s, &mut ["item"].iter()));

        let mut s = String::new();

        assert!(resource.resource_path_from_iter(
            &mut s,
            #[allow(clippy::useless_vec)]
            &mut vec!["item", "item2"].iter()
        ));
        assert_eq!(s, "/user/item/item2/");
    }

    #[test]
    fn multi_pattern_build_path() {
        let resource = ResourceDef::new(["/user/{id}", "/profile/{id}"]);
        let mut s = String::new();
        assert!(resource.resource_path_from_iter(&mut s, &mut ["123"].iter()));
        assert_eq!(s, "/user/123");
    }

    #[test]
    fn multi_pattern_capture_segment_values() {
        let resource = ResourceDef::new(["/user/{id}", "/profile/{id}"]);

        let mut path = Path::new("/user/123");
        assert!(resource.capture_match_info(&mut path));
        assert!(path.get("id").is_some());

        let mut path = Path::new("/profile/123");
        assert!(resource.capture_match_info(&mut path));
        assert!(path.get("id").is_some());

        let resource = ResourceDef::new(["/user/{id}", "/profile/{uid}"]);

        let mut path = Path::new("/user/123");
        assert!(resource.capture_match_info(&mut path));
        assert!(path.get("id").is_some());
        assert!(path.get("uid").is_none());

        let mut path = Path::new("/profile/123");
        assert!(resource.capture_match_info(&mut path));
        assert!(path.get("id").is_none());
        assert!(path.get("uid").is_some());
    }

    #[test]
    fn dynamic_prefix_proper_segmentation() {
        let resource = ResourceDef::prefix(r"/id/{id:\d{3}}");

        assert!(resource.is_match("/id/123"));
        assert!(resource.is_match("/id/123/foo"));
        assert!(!resource.is_match("/id/1234"));
        assert!(!resource.is_match("/id/123a"));

        assert_eq!(resource.find_match("/id/123"), Some(7));
        assert_eq!(resource.find_match("/id/123/foo"), Some(7));
        assert_eq!(resource.find_match("/id/1234"), None);
        assert_eq!(resource.find_match("/id/123a"), None);
    }

    #[test]
    fn build_path_map() {
        let resource = ResourceDef::new("/user/{item1}/{item2}/");

        let mut map = HashMap::new();
        map.insert("item1", "item");

        let mut s = String::new();
        assert!(!resource.resource_path_from_map(&mut s, &map));

        map.insert("item2", "item2");

        let mut s = String::new();
        assert!(resource.resource_path_from_map(&mut s, &map));
        assert_eq!(s, "/user/item/item2/");
    }

    #[test]
    fn build_path_tail() {
        let resource = ResourceDef::new("/user/{item1}*");

        let mut s = String::new();
        assert!(!resource.resource_path_from_iter(&mut s, &mut [""; 0].iter()));

        let mut s = String::new();
        assert!(resource.resource_path_from_iter(&mut s, &mut ["user1"].iter()));
        assert_eq!(s, "/user/user1");

        let mut s = String::new();
        let mut map = HashMap::new();
        map.insert("item1", "item");
        assert!(resource.resource_path_from_map(&mut s, &map));
        assert_eq!(s, "/user/item");
    }

    #[test]
    fn prefix_trailing_slash() {
        // The prefix "/abc/" matches two segments: ["user", ""]

        // These are not prefixes
        let re = ResourceDef::prefix("/abc/");
        assert_eq!(re.find_match("/abc/def"), None);
        assert_eq!(re.find_match("/abc//def"), Some(5));

        let re = ResourceDef::prefix("/{id}/");
        assert_eq!(re.find_match("/abc/def"), None);
        assert_eq!(re.find_match("/abc//def"), Some(5));
    }

    #[test]
    fn join() {
        // test joined defs match the same paths as each component separately

        fn seq_find_match(re1: &ResourceDef, re2: &ResourceDef, path: &str) -> Option<usize> {
            let len1 = re1.find_match(path)?;
            let len2 = re2.find_match(&path[len1..])?;
            Some(len1 + len2)
        }

        macro_rules! join_test {
            ($pat1:expr, $pat2:expr => $($test:expr),+) => {{
                let pat1 = $pat1;
                let pat2 = $pat2;
                $({
                    let _path = $test;
                    let (re1, re2) = (ResourceDef::prefix(pat1), ResourceDef::new(pat2));
                    let _seq = seq_find_match(&re1, &re2, _path);
                    let _join = re1.join(&re2).find_match(_path);
                    assert_eq!(
                        _seq, _join,
                        "patterns: prefix {:?}, {:?}; mismatch on \"{}\"; seq={:?}; join={:?}",
                        pat1, pat2, _path, _seq, _join
                    );
                    assert!(!re1.join(&re2).is_prefix());

                    let (re1, re2) = (ResourceDef::prefix(pat1), ResourceDef::prefix(pat2));
                    let _seq = seq_find_match(&re1, &re2, _path);
                    let _join = re1.join(&re2).find_match(_path);
                    assert_eq!(
                        _seq, _join,
                        "patterns: prefix {:?}, prefix {:?}; mismatch on \"{}\"; seq={:?}; join={:?}",
                        pat1, pat2, _path, _seq, _join
                    );
                    assert!(re1.join(&re2).is_prefix());
                })+
            }}
        }

        join_test!("", "" => "", "/hello", "/");
        join_test!("/user", "" => "", "/user", "/user/123", "/user11", "user", "user/123");
        join_test!("",  "/user" => "", "/user", "foo", "/user11", "user", "user/123");
        join_test!("/user",  "/xx" => "", "",  "/", "/user", "/xx", "/userxx", "/user/xx");

        join_test!(["/ver/{v}", "/v{v}"], ["/req/{req}", "/{req}"] => "/v1/abc", 
                   "/ver/1/abc", "/v1/req/abc", "/ver/1/req/abc", "/v1/abc/def",
                   "/ver1/req/abc/def", "", "/", "/v1/");
    }

    #[test]
    fn match_methods_agree() {
        macro_rules! match_methods_agree {
            ($pat:expr => $($test:expr),+) => {{
                match_methods_agree!(finish $pat, ResourceDef::new($pat), $($test),+);
            }};
            (prefix $pat:expr => $($test:expr),+) => {{
                match_methods_agree!(finish $pat, ResourceDef::prefix($pat), $($test),+);
            }};
            (finish $pat:expr, $re:expr, $($test:expr),+) => {{
                let re = $re;
                $({
                    let _is = re.is_match($test);
                    let _find = re.find_match($test).is_some();
                    assert_eq!(
                        _is, _find,
                        "pattern: {:?}; mismatch on \"{}\"; is={}; find={}",
                        $pat, $test, _is, _find
                    );
                })+
            }}
        }

        match_methods_agree!("" => "", "/", "/foo");
        match_methods_agree!("/" => "", "/", "/foo");
        match_methods_agree!("/user" => "user", "/user", "/users", "/user/123", "/foo");
        match_methods_agree!("/v{v}" => "v", "/v", "/v1", "/v222", "/foo");
        match_methods_agree!(["/v{v}", "/version/{v}"] => "/v", "/v1", "/version", "/version/1", "/foo");

        match_methods_agree!("/path{tail}*" => "/path", "/path1", "/path/123");
        match_methods_agree!("/path/{tail}*" => "/path", "/path1", "/path/123");

        match_methods_agree!(prefix "" => "", "/", "/foo");
        match_methods_agree!(prefix "/user" => "user", "/user", "/users", "/user/123", "/foo");
        match_methods_agree!(prefix r"/id/{id:\d{3}}" => "/id/123", "/id/1234");
        match_methods_agree!(["/v{v}", "/ver/{v}"] => "", "s/v", "/v1", "/v1/xx", "/ver/i3/5", "/ver/1");
    }

    #[test]
    #[should_panic]
    fn duplicate_segment_name() {
        ResourceDef::new("/user/{id}/post/{id}");
    }

    #[test]
    #[should_panic]
    fn invalid_dynamic_segment_delimiter() {
        ResourceDef::new("/user/{username");
    }

    #[test]
    #[should_panic]
    fn invalid_dynamic_segment_name() {
        ResourceDef::new("/user/{}");
    }

    #[test]
    #[should_panic]
    fn invalid_too_many_dynamic_segments() {
        // valid
        ResourceDef::new("/{a}/{b}/{c}/{d}/{e}/{f}/{g}/{h}/{i}/{j}/{k}/{l}/{m}/{n}/{o}/{p}");

        // panics
        ResourceDef::new("/{a}/{b}/{c}/{d}/{e}/{f}/{g}/{h}/{i}/{j}/{k}/{l}/{m}/{n}/{o}/{p}/{q}");
    }

    #[test]
    #[should_panic]
    fn invalid_custom_regex_for_tail() {
        ResourceDef::new(r"/{tail:\d+}*");
    }

    #[test]
    #[should_panic]
    fn invalid_unnamed_tail_segment() {
        ResourceDef::new("/*");
    }

    #[test]
    #[should_panic]
    fn prefix_plus_tail_match_disallowed() {
        ResourceDef::prefix("/user/{id}*");
    }
}
