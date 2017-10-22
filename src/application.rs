use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;

use task::Task;
use payload::Payload;
use route::{RouteHandler, FnHandler};
use resource::Resource;
use recognizer::{RouteRecognizer, check_pattern};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use server::HttpHandler;


/// Application
pub struct Application<S> {
    state: Rc<S>,
    prefix: String,
    default: Resource<S>,
    handlers: HashMap<String, Box<RouteHandler<S>>>,
    router: RouteRecognizer<Resource<S>>,
}

impl<S: 'static> HttpHandler for Application<S> {

    fn prefix(&self) -> &str {
        &self.prefix
    }
    
    fn handle(&self, req: HttpRequest, payload: Payload) -> Task {
        if let Some((params, h)) = self.router.recognize(req.path()) {
            if let Some(params) = params {
                h.handle(
                    req.with_match_info(params), payload, Rc::clone(&self.state))
            } else {
                h.handle(req, payload, Rc::clone(&self.state))
            }
        } else {
            for (prefix, handler) in &self.handlers {
                if req.path().starts_with(prefix) {
                    return handler.handle(req, payload, Rc::clone(&self.state))
                }
            }
            self.default.handle(req, payload, Rc::clone(&self.state))
        }
    }
}

impl Application<()> {

    /// Create default `ApplicationBuilder` with no state
    pub fn default<T: ToString>(prefix: T) -> ApplicationBuilder<()> {
        ApplicationBuilder {
            parts: Some(ApplicationBuilderParts {
                state: (),
                prefix: prefix.to_string(),
                default: Resource::default(),
                handlers: HashMap::new(),
                resources: HashMap::new()})
        }
    }
}

impl<S> Application<S> where S: 'static {

    /// Create application builder with specific state. State is shared with all
    /// routes within same application and could be
    /// accessed with `HttpContext::state()` method.
    pub fn builder<T: ToString>(prefix: T, state: S) -> ApplicationBuilder<S> {
        ApplicationBuilder {
            parts: Some(ApplicationBuilderParts {
                state: state,
                prefix: prefix.to_string(),
                default: Resource::default(),
                handlers: HashMap::new(),
                resources: HashMap::new()})
        }
    }
}

struct ApplicationBuilderParts<S> {
    state: S,
    prefix: String,
    default: Resource<S>,
    handlers: HashMap<String, Box<RouteHandler<S>>>,
    resources: HashMap<String, Resource<S>>,
}

/// Application builder
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
    /// in the `Params` object returned by `Request.match_info()` method.
    ///
    /// By default, each part matches the regular expression `[^{}/]+`.
    ///
    /// You can also specify a custom regex in the form `{identifier:regex}`:
    ///
    /// For instance, to route Get requests on any route matching `/users/{userid}/{friend}` and
    /// store userid and friend in the exposed Params object:
    ///
    /// ```rust
    /// extern crate actix;
    /// extern crate actix_web;
    ///
    /// use actix::*;
    /// use actix_web::*;
    ///
    /// struct MyRoute;
    ///
    /// impl Actor for MyRoute {
    ///     type Context = HttpContext<Self>;
    /// }
    ///
    /// impl Route for MyRoute {
    ///     type State = ();
    ///
    ///     fn request(req: HttpRequest,
    ///                payload: Payload,
    ///                ctx: &mut HttpContext<Self>) -> Reply<Self> {
    ///         Reply::reply(httpcodes::HTTPOk)
    ///     }
    /// }
    /// fn main() {
    ///     let app = Application::default("/")
    ///         .resource("/test", |r| {
    ///              r.get::<MyRoute>();
    ///              r.handler(Method::HEAD, |req, payload, state| {
    ///                  httpcodes::HTTPMethodNotAllowed
    ///              });
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn resource<F, P: ToString>(&mut self, path: P, f: F) -> &mut Self
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // add resource
            let path = path.to_string();
            if !parts.resources.contains_key(&path) {
                check_pattern(&path);
                parts.resources.insert(path.clone(), Resource::default());
            }
            f(parts.resources.get_mut(&path).unwrap());
        }
        self
    }

    /// Default resource is used if no matches route could be found.
    pub fn default_resource<F>(&mut self, f: F) -> &mut Self
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            f(&mut parts.default);
        }
        self
    }

    /// This method register handler for specified path.
    ///
    /// ```rust
    /// extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = Application::default("/")
    ///         .handler("/test", |req, payload, state| {
    ///              match *req.method() {
    ///                  Method::GET => httpcodes::HTTPOk,
    ///                  Method::POST => httpcodes::HTTPMethodNotAllowed,
    ///                  _ => httpcodes::HTTPNotFound,
    ///              }
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn handler<P, F, R>(&mut self, path: P, handler: F) -> &mut Self
        where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
              R: Into<HttpResponse> + 'static,
              P: ToString,
    {
        self.parts.as_mut().expect("Use after finish")
            .handlers.insert(path.to_string(), Box::new(FnHandler::new(handler)));
        self
    }

    /// Add path handler
    pub fn route_handler<H, P>(&mut self, path: P, h: H) -> &mut Self
        where H: RouteHandler<S> + 'static, P: ToString
    {
        {
            // add resource
            let parts = self.parts.as_mut().expect("Use after finish");
            let path = path.to_string();
            if parts.handlers.contains_key(&path) {
                panic!("Handler already registered: {:?}", path);
            }
            parts.handlers.insert(path, Box::new(h));
        }
        self
    }

    /// Construct application
    pub fn finish(&mut self) -> Application<S> {
        let parts = self.parts.take().expect("Use after finish");

        let mut handlers = HashMap::new();
        let prefix = if parts.prefix.ends_with('/') {
            parts.prefix
        } else {
            parts.prefix + "/"
        };

        let mut routes = Vec::new();
        for (path, handler) in parts.resources {
            routes.push((path, handler))
        }

        for (path, mut handler) in parts.handlers {
            let path = prefix.clone() + path.trim_left_matches('/');
            handler.set_prefix(path.clone());
            handlers.insert(path, handler);
        }
        Application {
            state: Rc::new(parts.state),
            prefix: prefix.clone(),
            default: parts.default,
            handlers: handlers,
            router: RouteRecognizer::new(prefix, routes) }
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
