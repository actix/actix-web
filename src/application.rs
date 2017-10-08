use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;

use route_recognizer::Router;

use task::Task;
use route::{Payload, RouteHandler};
use router::Handler;
use resource::Resource;
use httpmessage::HttpRequest;


/// Application
pub struct Application<S=()> {
    state: S,
    default: Resource<S>,
    resources: HashMap<String, Resource<S>>,
}

impl<S> Application<S> where S: 'static
{
    pub(crate) fn prepare(self, prefix: String) -> Box<Handler> {
        let mut router = Router::new();
        let prefix = if prefix.ends_with('/') {prefix } else { prefix + "/" };

        for (path, handler) in self.resources {
            let path = prefix.clone() + path.trim_left_matches('/');
            router.add(path.as_str(), handler);
        }

        Box::new(
            InnerApplication {
                state: Rc::new(self.state),
                default: self.default,
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

    /// Default resource is used if no matches route could be found.
    pub fn default_resource(&mut self) -> &mut Resource<S> {
        &mut self.default
    }
}


pub(crate)
struct InnerApplication<S> {
    state: Rc<S>,
    default: Resource<S>,
    router: Router<Resource<S>>,
}


impl<S: 'static> Handler for InnerApplication<S> {

    fn handle(&self, req: HttpRequest, payload: Option<Payload>) -> Task {
        if let Ok(h) = self.router.recognize(req.path()) {
            h.handler.handle(req.with_params(h.params), payload, Rc::clone(&self.state))
        } else {
            self.default.handle(req, payload, Rc::clone(&self.state))
        }
    }
}
