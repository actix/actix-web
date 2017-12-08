use regex::RegexSet;

pub struct RouteRecognizer<T> {
    re: RegexSet,
    routes: Vec<T>,
}

impl<T> RouteRecognizer<T> {

    pub fn new<U, K>(routes: U) -> Self
        where U: IntoIterator<Item=(K, T)>, K: Into<String>,
    {
        let mut paths = Vec::new();
        let mut routes = Vec::new();
        for item in routes {
            let pattern = parse(&item.0.into());
            paths.push(pattern);
            routes.push(item.1);
        };
        let regset = RegexSet::new(&paths);

        RouteRecognizer {
            re: regset.unwrap(),
            routes: routes,
        }
    }

    pub fn recognize(&self, path: &str) -> Option<&T> {
        if path.is_empty() {
            if let Some(idx) = self.re.matches("/").into_iter().next() {
                return Some(&self.routes[idx])
            }
        } else if let Some(idx) = self.re.matches(path).into_iter().next() {
            return Some(&self.routes[idx])
        }
        None
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

#[cfg(test)]
mod tests {
    use regex::Regex;
    use super::*;
    use std::iter::FromIterator;

    #[test]
    fn test_recognizer() {
        let routes = vec![
            ("/name", None, 1),
            ("/name/{val}", None, 2),
            ("/name/{val}/index.html", None, 3),
            ("/v{val}/{val2}/index.html", None, 4),
            ("/v/{tail:.*}", None, 5),
        ];
        let rec = RouteRecognizer::new("", routes);

        let (params, val) = rec.recognize("/name").unwrap();
        assert_eq!(*val, 1);
        assert!(params.unwrap().is_empty());

        let (params, val) = rec.recognize("/name/value").unwrap();
        assert_eq!(*val, 2);
        assert!(!params.as_ref().unwrap().is_empty());
        assert_eq!(params.as_ref().unwrap().get("val").unwrap(), "value");
        assert_eq!(&params.as_ref().unwrap()["val"], "value");

        let (params, val) = rec.recognize("/name/value2/index.html").unwrap();
        assert_eq!(*val, 3);
        assert!(!params.as_ref().unwrap().is_empty());
        assert_eq!(params.as_ref().unwrap().get("val").unwrap(), "value2");
        assert_eq!(params.as_ref().unwrap().by_idx(0).unwrap(), "value2");

        let (params, val) = rec.recognize("/vtest/ttt/index.html").unwrap();
        assert_eq!(*val, 4);
        assert!(!params.as_ref().unwrap().is_empty());
        assert_eq!(params.as_ref().unwrap().get("val").unwrap(), "test");
        assert_eq!(params.as_ref().unwrap().get("val2").unwrap(), "ttt");
        assert_eq!(params.as_ref().unwrap().by_idx(0).unwrap(), "test");
        assert_eq!(params.as_ref().unwrap().by_idx(1).unwrap(), "ttt");

        let (params, val) = rec.recognize("/v/blah-blah/index.html").unwrap();
        assert_eq!(*val, 5);
        assert!(!params.as_ref().unwrap().is_empty());
        assert_eq!(params.as_ref().unwrap().get("tail").unwrap(), "blah-blah/index.html");
    }

    fn assert_parse(pattern: &str, expected_re: &str) -> Regex {
        let (re_str, _) = parse(pattern);
        assert_eq!(&*re_str, expected_re);
        Regex::new(&re_str).unwrap()
    }

    #[test]
    fn test_parse_static() {
        let re = assert_parse("/", r"^/$");
        assert!(re.is_match("/"));
        assert!(!re.is_match("/a"));

        let re = assert_parse("/name", r"^/name$");
        assert!(re.is_match("/name"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name/"));
        assert!(!re.is_match("/name~"));

        let re = assert_parse("/name/", r"^/name/$");
        assert!(re.is_match("/name/"));
        assert!(!re.is_match("/name"));
        assert!(!re.is_match("/name/gs"));

        let re = assert_parse("/user/profile", r"^/user/profile$");
        assert!(re.is_match("/user/profile"));
        assert!(!re.is_match("/user/profile/profile"));
    }

    #[test]
    fn test_parse_param() {
        let re = assert_parse("/user/{id}", r"^/user/(?P<id>[^/]+)$");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let captures = re.captures("/user/profile").unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "profile");
        assert_eq!(captures.name("id").unwrap().as_str(), "profile");

        let captures = re.captures("/user/1245125").unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "1245125");
        assert_eq!(captures.name("id").unwrap().as_str(), "1245125");

        let re = assert_parse(
            "/v{version}/resource/{id}",
            r"^/v(?P<version>[^/]+)/resource/(?P<id>[^/]+)$",
        );
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let captures = re.captures("/v151/resource/adahg32").unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "151");
        assert_eq!(captures.name("version").unwrap().as_str(), "151");
        assert_eq!(captures.name("id").unwrap().as_str(), "adahg32");
    }
}
