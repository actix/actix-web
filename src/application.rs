use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;

use route_recognizer::Router;

use task::Task;
use route::RouteHandler;
use router::Handler;
use resource::Resource;
use payload::Payload;
use httprequest::HttpRequest;


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

impl Default for Application<()> {

    /// Create default `Application` with no state
    fn default() -> Self {
        Application {
            state: (),
            default: Resource::default(),
            handlers: HashMap::new(),
            resources: HashMap::new(),
        }
    }
}

impl<S> Application<S> where S: 'static {

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
    pub fn add<P: ToString>(&mut self, path: P) -> &mut Resource<S>
    {
        let path = path.to_string();

        // add resource
        if !self.resources.contains_key(&path) {
            self.resources.insert(path.clone(), Resource::default());
        }

        self.resources.get_mut(&path).unwrap()
    }

    /// Add path handler
    pub fn add_handler<H, P>(&mut self, path: P, h: H)
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
            h.handler.handle(req.with_params(h.params), payload, Rc::clone(&self.state))
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
