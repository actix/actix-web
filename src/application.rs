use std::rc::Rc;
use std::collections::HashMap;

use task::Task;
use route::{RouteHandler, FnHandler};
use resource::Resource;
use recognizer::{RouteRecognizer, check_pattern};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use channel::HttpHandler;
use pipeline::Pipeline;
use middlewares::Middleware;


/// Application
pub struct Application<S> {
    state: Rc<S>,
    prefix: String,
    default: Resource<S>,
    handlers: HashMap<String, Box<RouteHandler<S>>>,
    router: RouteRecognizer<Resource<S>>,
    middlewares: Rc<Vec<Box<Middleware>>>,
}

impl<S: 'static> Application<S> {

    fn run(&self, mut req: HttpRequest) -> Task {
        if let Some((params, h)) = self.router.recognize(req.path()) {
            if let Some(params) = params {
                req.set_match_info(params);
            }
            h.handle(req, Rc::clone(&self.state))
        } else {
            for (prefix, handler) in &self.handlers {
                if req.path().starts_with(prefix) {
                    return handler.handle(req, Rc::clone(&self.state))
                }
            }
            self.default.handle(req, Rc::clone(&self.state))
        }
    }
}

impl<S: 'static> HttpHandler for Application<S> {

    fn prefix(&self) -> &str {
        &self.prefix
    }
    
    fn handle(&self, req: HttpRequest) -> Pipeline {
        Pipeline::new(req, Rc::clone(&self.middlewares), &|req: HttpRequest| {self.run(req)})
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
                handlers: HashMap::new(),
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
                handlers: HashMap::new(),
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
    handlers: HashMap<String, Box<RouteHandler<S>>>,
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
    ///     fn request(req: HttpRequest, ctx: &mut HttpContext<Self>) -> RouteResult<Self> {
    ///         Reply::reply(httpcodes::HTTPOk)
    ///     }
    /// }
    /// fn main() {
    ///     let app = Application::default("/")
    ///         .resource("/test", |r| {
    ///              r.get::<MyRoute>();
    ///              r.handler(Method::HEAD, |req, state| {
    ///                  Ok(httpcodes::HTTPMethodNotAllowed)
    ///              });
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

    /// This method register handler for specified path prefix.
    /// Any path that starts with this prefix matches handler.
    ///
    /// ```rust
    /// extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = Application::default("/")
    ///         .handler("/test", |req, state| {
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
        where F: Fn(HttpRequest, &S) -> R + 'static,
              R: Into<HttpResponse> + 'static,
              P: Into<String>,
    {
        self.parts.as_mut().expect("Use after finish")
            .handlers.insert(path.into(), Box::new(FnHandler::new(handler)));
        self
    }

    /// Add path handler
    pub fn route_handler<H, P>(&mut self, path: P, h: H) -> &mut Self
        where H: RouteHandler<S> + 'static, P: Into<String>
    {
        {
            // add resource
            let parts = self.parts.as_mut().expect("Use after finish");
            let path = path.into();
            if parts.handlers.contains_key(&path) {
                panic!("Handler already registered: {:?}", path);
            }
            parts.handlers.insert(path, Box::new(h));
        }
        self
    }

    /// Construct application
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
            router: RouteRecognizer::new(prefix, routes),
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
