use std::rc::Rc;
use std::collections::HashMap;

use error::UriGenerationError;
use handler::{Reply, RouteHandler};
use route::Route;
use resource::Resource;
use recognizer::{RouteRecognizer, check_pattern, PatternElement};
use httprequest::HttpRequest;
use channel::HttpHandler;
use pipeline::Pipeline;
use middlewares::Middleware;

pub struct Router<S>(Rc<RouteRecognizer<Resource<S>>>);

impl<S: 'static> Router<S> {
    pub fn new(prefix: String, map: HashMap<String, Resource<S>>) -> Router<S>
    {
        let mut resources = Vec::new();
        for (path, resource) in map {
            resources.push((path, resource.get_name(), resource))
        }

        Router(Rc::new(RouteRecognizer::new(prefix, resources)))
    }

    pub fn has_route(&self, path: &str) -> bool {
        self.0.recognize(path).is_some()
    }

    pub fn resource_path<'a, U>(&self, prefix: &str, name: &str, elements: U)
                                -> Result<String, UriGenerationError>
        where U: IntoIterator<Item=&'a str>
    {
        if let Some(pattern) = self.0.get_pattern(name) {
            let mut iter = elements.into_iter();
            let mut vec = vec![prefix];
            for el in pattern.elements() {
                match *el {
                    PatternElement::Str(ref s) => vec.push(s),
                    PatternElement::Var(_) => {
                        if let Some(val) = iter.next() {
                            vec.push(val)
                        } else {
                            return Err(UriGenerationError::NotEnoughElements)
                        }
                    }
                }
            }
            let s = vec.join("/").to_owned();
            Ok(s)
        } else {
            Err(UriGenerationError::ResourceNotFound)
        }
    }
}

/// Application
pub struct Application<S> {
    state: Rc<S>,
    prefix: String,
    default: Resource<S>,
    routes: Vec<(String, Route<S>)>,
    router: Router<S>,
    middlewares: Rc<Vec<Box<Middleware>>>,
}

impl<S: 'static> Application<S> {

    fn run(&self, req: HttpRequest) -> Reply {
        let mut req = req.with_state(Rc::clone(&self.state));

        if let Some((params, h)) = self.router.0.recognize(req.path()) {
            if let Some(params) = params {
                req.set_match_info(params);
            }
            h.handle(req)
        } else {
            for route in &self.routes {
                if req.path().starts_with(&route.0) && route.1.check(&mut req) {
                    req.set_prefix(route.0.len());
                    return route.1.handle(req)
                }
            }
            self.default.handle(req)
        }
    }
}

impl<S: 'static> HttpHandler for Application<S> {

    fn handle(&self, req: HttpRequest) -> Result<Pipeline, HttpRequest> {
        if req.path().starts_with(&self.prefix) {
            Ok(Pipeline::new(req, Rc::clone(&self.middlewares),
                             &|req: HttpRequest| self.run(req)))
        } else {
            Err(req)
        }
    }
}

impl Application<()> {

    /// Create default `ApplicationBuilder` with no state
    pub fn default<T: Into<String>>(prefix: T) -> ApplicationBuilder<()> {
        ApplicationBuilder {
            parts: Some(ApplicationBuilderParts {
                state: (),
                prefix: prefix.into(),
                default: Resource::default_not_found(),
                routes: Vec::new(),
                resources: HashMap::new(),
                middlewares: Vec::new(),
            })
        }
    }
}

impl<S> Application<S> where S: 'static {

    /// Create application builder with specific state. State is shared with all
    /// routes within same application and could be
    /// accessed with `HttpContext::state()` method.
    pub fn build<T: Into<String>>(prefix: T, state: S) -> ApplicationBuilder<S> {
        ApplicationBuilder {
            parts: Some(ApplicationBuilderParts {
                state: state,
                prefix: prefix.into(),
                default: Resource::default_not_found(),
                routes: Vec::new(),
                resources: HashMap::new(),
                middlewares: Vec::new(),
            })
        }
    }
}

struct ApplicationBuilderParts<S> {
    state: S,
    prefix: String,
    default: Resource<S>,
    routes: Vec<(String, Route<S>)>,
    resources: HashMap<String, Resource<S>>,
    middlewares: Vec<Box<Middleware>>,
}

/// Structure that follows the builder pattern for building `Application` structs.
pub struct ApplicationBuilder<S=()> {
    parts: Option<ApplicationBuilderParts<S>>,
}

impl<S> ApplicationBuilder<S> where S: 'static {

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
    /// extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = Application::default("/")
    ///         .resource("/test", |r| {
    ///              r.method(Method::GET).f(|_| httpcodes::HTTPOk);
    ///              r.method(Method::HEAD).f(|_| httpcodes::HTTPMethodNotAllowed);
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn resource<F, P: Into<String>>(&mut self, path: P, f: F) -> &mut Self
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // add resource
            let path = path.into();
            if !parts.resources.contains_key(&path) {
                check_pattern(&path);
                parts.resources.insert(path.clone(), Resource::default());
            }
            f(parts.resources.get_mut(&path).unwrap());
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

    /// Register a middleware
    pub fn middleware<T>(&mut self, mw: T) -> &mut Self
        where T: Middleware + 'static
    {
        self.parts.as_mut().expect("Use after finish")
            .middlewares.push(Box::new(mw));
        self
    }

    /// Construct application
    pub fn finish(&mut self) -> Application<S> {
        let parts = self.parts.take().expect("Use after finish");

        let prefix = if parts.prefix.ends_with('/') {
            parts.prefix
        } else {
            parts.prefix + "/"
        };

        let mut routes = Vec::new();
        for (path, route) in parts.routes {
            routes.push((prefix.clone() + path.trim_left_matches('/'), route));
        }
        Application {
            state: Rc::new(parts.state),
            prefix: prefix.clone(),
            default: parts.default,
            routes: routes,
            router: Router::new(prefix, parts.resources),
            middlewares: Rc::new(parts.middlewares),
        }
    }
}

impl<S: 'static> From<ApplicationBuilder<S>> for Application<S> {
    fn from(mut builder: ApplicationBuilder<S>) -> Application<S> {
        builder.finish()
    }
}

impl<S: 'static> Iterator for ApplicationBuilder<S> {
    type Item = Application<S>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.parts.is_some() {
            Some(self.finish())
        } else {
            None
        }
    }
}
