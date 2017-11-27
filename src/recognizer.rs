use std::rc::Rc;
use std::collections::HashMap;

use regex::{Regex, RegexSet, Captures};


#[doc(hidden)]
pub struct RouteRecognizer<T> {
    prefix: usize,
    patterns: RegexSet,
    routes: Vec<(Pattern, T)>,
}

impl<T> Default for RouteRecognizer<T> {

    fn default() -> Self {
        RouteRecognizer {
            prefix: 0,
            patterns: RegexSet::new([""].iter()).unwrap(),
            routes: Vec::new(),
        }
    }
}

impl<T> RouteRecognizer<T> {

    pub fn new<P: Into<String>, U>(prefix: P, routes: U) -> Self
        where U: IntoIterator<Item=(String, T)>
    {
        let mut paths = Vec::new();
        let mut handlers = Vec::new();
        for item in routes {
            let pat = parse(&item.0);
            handlers.push((Pattern::new(&pat), item.1));
            paths.push(pat);
        };
        let regset = RegexSet::new(&paths);

        RouteRecognizer {
            prefix: prefix.into().len() - 1,
            patterns: regset.unwrap(),
            routes: handlers,
        }
    }

    pub fn set_routes(&mut self, routes: Vec<(&str, T)>) {
        let mut paths = Vec::new();
        let mut handlers = Vec::new();
        for item in routes {
            let pat = parse(item.0);
            handlers.push((Pattern::new(&pat), item.1));
            paths.push(pat);
        };
        self.patterns = RegexSet::new(&paths).unwrap();
        self.routes = handlers;
    }

    pub fn set_prefix<P: Into<String>>(&mut self, prefix: P) {
        let p = prefix.into();
        if p.ends_with('/') {
            self.prefix = p.len() - 1;
        } else {
            self.prefix = p.len();
        }
    }

    pub fn recognize(&self, path: &str) -> Option<(Option<Params>, &T)> {
        let p = &path[self.prefix..];
        if p.is_empty() {
            if let Some(idx) = self.patterns.matches("/").into_iter().next() {
                let (ref pattern, ref route) = self.routes[idx];
                return Some((pattern.match_info(&path[self.prefix..]), route))
            }
        } else if let Some(idx) = self.patterns.matches(p).into_iter().next() {
            let (ref pattern, ref route) = self.routes[idx];
            return Some((pattern.match_info(&path[self.prefix..]), route))
        }
        None
    }
}

struct Pattern {
    re: Regex,
    names: Rc<HashMap<String, usize>>,
}

impl Pattern {
    fn new(pattern: &str) -> Self {
        let re = Regex::new(pattern).unwrap();
        let names = re.capture_names()
            .enumerate()
            .filter_map(|(i, name)| name.map(|name| (name.to_owned(), i)))
            .collect();

        Pattern {
            re,
            names: Rc::new(names),
        }
    }

    fn match_info(&self, text: &str) -> Option<Params> {
        let captures = match self.re.captures(text) {
            Some(captures) => captures,
            None => return None,
        };

        Some(Params::new(Rc::clone(&self.names), text, &captures))
    }
}

pub(crate) fn check_pattern(path: &str) {
    if let Err(err) = Regex::new(&parse(path)) {
        panic!("Wrong path pattern: \"{}\" {}", path, err);
    }
}

fn parse(pattern: &str) -> String {
    const DEFAULT_PATTERN: &str = "[^/]+";

    let mut re = String::from("^/");
    let mut in_param = false;
    let mut in_param_pattern = false;
    let mut param_name = String::new();
    let mut param_pattern = String::from(DEFAULT_PATTERN);

    for (index, ch) in pattern.chars().enumerate() {
        // All routes must have a leading slash so its optional to have one
        if index == 0 && ch == '/' {
            continue;
        }

        if in_param {
            // In parameter segment: `{....}`
            if ch == '}' {
                re.push_str(&format!(r"(?P<{}>{})", &param_name, &param_pattern));

                param_name.clear();
                param_pattern = String::from(DEFAULT_PATTERN);

                in_param_pattern = false;
                in_param = false;
            } else if ch == ':' {
                // The parameter name has been determined; custom pattern land
                in_param_pattern = true;
                param_pattern.clear();
            } else if in_param_pattern {
                // Ignore leading whitespace for pattern
                if !(ch == ' ' && param_pattern.is_empty()) {
                    param_pattern.push(ch);
                }
            } else {
                param_name.push(ch);
            }
        } else if ch == '{' {
            in_param = true;
        } else {
            re.push(ch);
        }
    }

    re.push('$');
    re
}

/// Route match information
///
/// If resource path contains variable patterns, `Params` stores this variables.
#[derive(Debug)]
pub struct Params {
    text: String,
    matches: Vec<Option<(usize, usize)>>,
    names: Rc<HashMap<String, usize>>,
}

impl Params {
    pub(crate) fn new(names: Rc<HashMap<String, usize>>,
                      text: &str,
                      captures: &Captures) -> Self
    {
        Params {
            names,
            text: text.into(),
            matches: captures
                .iter()
                .map(|capture| capture.map(|m| (m.start(), m.end())))
                .collect(),
        }
    }

    pub(crate) fn empty() -> Self
    {
        Params {
            text: String::new(),
            names: Rc::new(HashMap::new()),
            matches: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn by_idx(&self, index: usize) -> Option<&str> {
        self.matches
            .get(index + 1)
            .and_then(|m| m.map(|(start, end)| &self.text[start..end]))
    }

    /// Get matched parameter by name
    pub fn get(&self, key: &str) -> Option<&str> {
        self.names.get(key).and_then(|&i| self.by_idx(i - 1))
    }
}
