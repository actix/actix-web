use std::rc::Rc;
use std::collections::HashMap;

use handler::{Reply, RouteHandler};
use router::{Router, Pattern};
use resource::Resource;
use httprequest::HttpRequest;
use channel::{HttpHandler, IntoHttpHandler};
use pipeline::Pipeline;
use middlewares::Middleware;

/// Application
pub struct HttpApplication<S> {
    state: Rc<S>,
    prefix: String,
    default: Resource<S>,
    router: Router<S>,
    middlewares: Rc<Vec<Box<Middleware>>>,
}

impl<S: 'static> HttpApplication<S> {

    fn run(&self, req: HttpRequest) -> Reply {
        let mut req = req.with_state(Rc::clone(&self.state), self.router.clone());

        if let Some(h) = self.router.recognize(&mut req) {
            h.handle(req)
        } else {
            self.default.handle(req)
        }
    }
}

impl<S: 'static> HttpHandler for HttpApplication<S> {

    fn handle(&self, req: HttpRequest) -> Result<Pipeline, HttpRequest> {
        if req.path().starts_with(&self.prefix) {
            Ok(Pipeline::new(req, Rc::clone(&self.middlewares),
                             &|req: HttpRequest| self.run(req)))
        } else {
            Err(req)
        }
    }
}

struct ApplicationParts<S> {
    state: S,
    prefix: String,
    default: Resource<S>,
    resources: HashMap<Pattern, Option<Resource<S>>>,
    external: HashMap<String, Pattern>,
    middlewares: Vec<Box<Middleware>>,
}

/// Structure that follows the builder pattern for building `Application` structs.
pub struct Application<S=()> {
    parts: Option<ApplicationParts<S>>,
}

impl Application<()> {

    /// Create application with empty state. Application can
    /// be configured with builder-like pattern.
    ///
    /// This method accepts path prefix for which it should serve requests.
    pub fn new<T: Into<String>>(prefix: T) -> Application<()> {
        Application {
            parts: Some(ApplicationParts {
                state: (),
                prefix: prefix.into(),
                default: Resource::default_not_found(),
                resources: HashMap::new(),
                external: HashMap::new(),
                middlewares: Vec::new(),
            })
        }
    }
}

impl<S> Application<S> where S: 'static {

    /// Create application with specific state. Application can be
    /// configured with builder-like pattern.
    ///
    /// State is shared with all reousrces within same application and could be
    /// accessed with `HttpRequest::state()` method.
    pub fn with_state<T: Into<String>>(prefix: T, state: S) -> Application<S> {
        Application {
            parts: Some(ApplicationParts {
                state: state,
                prefix: prefix.into(),
                default: Resource::default_not_found(),
                resources: HashMap::new(),
                external: HashMap::new(),
                middlewares: Vec::new(),
            })
        }
    }

    /// Configure resource for specific path.
    ///
    /// Resource may have variable path also. For instance, a resource with
    /// the path */a/{name}/c* would match all incoming requests with paths
    /// such as */a/b/c*, */a/1/c*, and */a/etc/c*.
    ///
    /// A variable part is specified in the form `{identifier}`, where
    /// the identifier can be used later in a request handler to access the matched
    /// value for that part. This is done by looking up the identifier
    /// in the `Params` object returned by `HttpRequest.match_info()` method.
    ///
    /// By default, each part matches the regular expression `[^{}/]+`.
    ///
    /// You can also specify a custom regex in the form `{identifier:regex}`:
    ///
    /// For instance, to route Get requests on any route matching `/users/{userid}/{friend}` and
    /// store userid and friend in the exposed Params object:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = Application::new("/")
    ///         .resource("/test", |r| {
    ///              r.method(Method::GET).f(|_| httpcodes::HTTPOk);
    ///              r.method(Method::HEAD).f(|_| httpcodes::HTTPMethodNotAllowed);
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn resource<F>(&mut self, path: &str, f: F) -> &mut Self
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // add resource
            let mut resource = Resource::default();
            f(&mut resource);

            let pattern = Pattern::new(resource.get_name(), path);
            if parts.resources.contains_key(&pattern) {
                panic!("Resource {:?} is registered.", path);
            }

            parts.resources.insert(pattern, Some(resource));
        }
        self
    }

    /// Default resource is used if no match route could be found.
    pub fn default_resource<F>(&mut self, f: F) -> &mut Self
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            f(&mut parts.default);
        }
        self
    }

    /// Register external resource.
    ///
    /// External resources are useful for URL generation purposes only and
    /// are never considered for matching at request time.
    /// Call to `HttpRequest::url_for()` will work as expected.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn index(mut req: HttpRequest) -> Result<HttpResponse> {
    ///    let url = req.url_for("youtube", &["oHg5SJYRHA0"])?;
    ///    assert_eq!(url.as_str(), "https://youtube.com/watch/oHg5SJYRHA0");
    ///    Ok(httpcodes::HTTPOk.into())
    /// }
    ///
    /// fn main() {
    ///     let app = Application::new("/")
    ///         .resource("/index.html", |r| r.f(index))
    ///         .external_resource("youtube", "https://youtube.com/watch/{video_id}")
    ///         .finish();
    /// }
    /// ```
    pub fn external_resource<T, U>(&mut self, name: T, url: U) -> &mut Self
        where T: AsRef<str>, U: AsRef<str>
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            if parts.external.contains_key(name.as_ref()) {
                panic!("External resource {:?} is registered.", name.as_ref());
            }
            parts.external.insert(
                String::from(name.as_ref()), Pattern::new(name.as_ref(), url.as_ref()));
        }
        self
    }

    /// Register a middleware
    pub fn middleware<T>(&mut self, mw: T) -> &mut Self
        where T: Middleware + 'static
    {
        self.parts.as_mut().expect("Use after finish")
            .middlewares.push(Box::new(mw));
        self
    }

    /// Finish application configuration and create HttpHandler object
    pub fn finish(&mut self) -> HttpApplication<S> {
        let parts = self.parts.take().expect("Use after finish");
        let prefix = parts.prefix.trim().trim_right_matches('/');

        let mut resources = parts.resources;
        for (_, pattern) in parts.external {
            resources.insert(pattern, None);
        }

        HttpApplication {
            state: Rc::new(parts.state),
            prefix: prefix.to_owned(),
            default: parts.default,
            router: Router::new(prefix, resources),
            middlewares: Rc::new(parts.middlewares),
        }
    }
}

impl<S: 'static> IntoHttpHandler for Application<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(mut self) -> HttpApplication<S> {
        self.finish()
    }
}

impl<'a, S: 'static> IntoHttpHandler for &'a mut Application<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(self) -> HttpApplication<S> {
        self.finish()
    }
}

#[doc(hidden)]
impl<S: 'static> Iterator for Application<S> {
    type Item = HttpApplication<S>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.parts.is_some() {
            Some(self.finish())
        } else {
            None
        }
    }
}


#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use http::{Method, Version, Uri, HeaderMap, StatusCode};
    use super::*;
    use handler::ReplyItem;
    use httprequest::HttpRequest;
    use httpresponse::HttpResponse;
    use payload::Payload;
    use httpcodes;

    impl Reply {
        fn msg(self) -> Option<HttpResponse> {
            match self.into() {
                ReplyItem::Message(resp) => Some(resp),
                _ => None,
            }
        }
    }

    #[test]
    fn test_default_resource() {
        let app = Application::new("/")
            .resource("/test", |r| r.h(httpcodes::HTTPOk))
            .finish();

        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/test").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        let resp = app.run(req).msg().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/blah").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        let resp = app.run(req).msg().unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let app = Application::new("/")
            .default_resource(|r| r.h(httpcodes::HTTPMethodNotAllowed))
            .finish();
        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/blah").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        let resp = app.run(req).msg().unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_unhandled_prefix() {
        let app = Application::new("/test")
            .resource("/test", |r| r.h(httpcodes::HTTPOk))
            .finish();
        assert!(app.handle(HttpRequest::default()).is_err());
    }

    #[test]
    fn test_state() {
        let app = Application::with_state("/", 10)
            .resource("/", |r| r.h(httpcodes::HTTPOk))
            .finish();
        assert_eq!(
            app.run(HttpRequest::default()).msg().unwrap().status(), StatusCode::OK);
    }
}
