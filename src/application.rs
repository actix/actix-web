use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;

use route_recognizer::Router;

use task::Task;
use route::{RouteHandler, FnHandler};
use router::Handler;
use resource::Resource;
use payload::Payload;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;


/// Application
pub struct Application<S=()> {
    state: S,
    default: Resource<S>,
    handlers: HashMap<String, Box<RouteHandler<S>>>,
    resources: HashMap<String, Resource<S>>,
}

impl<S> Application<S> where S: 'static
{
    pub(crate) fn prepare(self, prefix: String) -> Box<Handler> {
        let mut router = Router::new();
        let mut handlers = HashMap::new();
        let prefix = if prefix.ends_with('/') {prefix } else { prefix + "/" };

        for (path, handler) in self.resources {
            let path = prefix.clone() + path.trim_left_matches('/');
            router.add(path.as_str(), handler);
        }

        for (path, mut handler) in self.handlers {
            let path = prefix.clone() + path.trim_left_matches('/');
            handler.set_prefix(path.clone());
            handlers.insert(path, handler);
        }
        Box::new(
            InnerApplication {
                state: Rc::new(self.state),
                default: self.default,
                handlers: handlers,
                router: router }
        )
    }
}


impl Application<()> {

    /// Create default `ApplicationBuilder` with no state
    pub fn default() -> ApplicationBuilder<()> {
        ApplicationBuilder {
            parts: Some(ApplicationBuilderParts {
                state: (),
                default: Resource::default(),
                handlers: HashMap::new(),
                resources: HashMap::new()})
        }
    }
}

impl<S> Application<S> where S: 'static {

    /// Create application builder
    pub fn builder(state: S) -> ApplicationBuilder<S> {
        ApplicationBuilder {
            parts: Some(ApplicationBuilderParts {
                state: state,
                default: Resource::default(),
                handlers: HashMap::new(),
                resources: HashMap::new()})
        }
    }

    /// Create http application with specific state. State is shared with all
    /// routes within same application and could be
    /// accessed with `HttpContext::state()` method.
    pub fn new(state: S) -> Application<S> {
        Application {
            state: state,
            default: Resource::default(),
            handlers: HashMap::new(),
            resources: HashMap::new(),
        }
    }

    /// Add resource by path.
    pub fn resource<P: ToString>(&mut self, path: P) -> &mut Resource<S>
    {
        let path = path.to_string();

        // add resource
        if !self.resources.contains_key(&path) {
            self.resources.insert(path.clone(), Resource::default());
        }

        self.resources.get_mut(&path).unwrap()
    }

    /// This method register handler for specified path.
    ///
    /// ```rust
    /// extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let mut app = Application::new(());
    ///
    ///     app.handler("/test", |req, payload, state| {
    ///          httpcodes::HTTPOk
    ///     });
    /// }
    /// ```
    pub fn handler<P, F, R>(&mut self, path: P, handler: F) -> &mut Self
        where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
              R: Into<HttpResponse> + 'static,
              P: ToString,
    {
        self.handlers.insert(path.to_string(), Box::new(FnHandler::new(handler)));
        self
    }

    /// Add path handler
    pub fn route_handler<H, P>(&mut self, path: P, h: H)
        where H: RouteHandler<S> + 'static, P: ToString
    {
        let path = path.to_string();

        // add resource
        if self.handlers.contains_key(&path) {
            panic!("Handler already registered: {:?}", path);
        }

        self.handlers.insert(path, Box::new(h));
    }

    /// Default resource is used if no matches route could be found.
    pub fn default_resource(&mut self) -> &mut Resource<S> {
        &mut self.default
    }
}

struct ApplicationBuilderParts<S> {
    state: S,
    default: Resource<S>,
    handlers: HashMap<String, Box<RouteHandler<S>>>,
    resources: HashMap<String, Resource<S>>,
}

impl<S> From<ApplicationBuilderParts<S>> for Application<S> {
    fn from(b: ApplicationBuilderParts<S>) -> Self {
        Application {
            state: b.state,
            default: b.default,
            handlers: b.handlers,
            resources: b.resources,
        }
    }
}

/// Application builder
pub struct ApplicationBuilder<S=()> {
    parts: Option<ApplicationBuilderParts<S>>,
}

impl<S> ApplicationBuilder<S> where S: 'static {

    /// Configure resource for specific path.
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
    ///     let app = Application::default()
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
    ///     let app = Application::default()
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
        self.parts.take().expect("Use after finish").into()
    }
}

pub(crate)
struct InnerApplication<S> {
    state: Rc<S>,
    default: Resource<S>,
    handlers: HashMap<String, Box<RouteHandler<S>>>,
    router: Router<Resource<S>>,
}


impl<S: 'static> Handler for InnerApplication<S> {

    fn handle(&self, req: HttpRequest, payload: Payload) -> Task {
        if let Ok(h) = self.router.recognize(req.path()) {
            h.handler.handle(
                req.with_match_info(h.params), payload, Rc::clone(&self.state))
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
