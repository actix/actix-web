use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;

use route_recognizer::Router;

use task::Task;
use route::{Payload, RouteHandler};
use router::HttpHandler;
use resource::HttpResource;
use httpmessage::HttpRequest;


/// Application
pub struct HttpApplication<S=()> {
    state: S,
    default: HttpResource<S>,
    resources: HashMap<String, HttpResource<S>>,
}

impl<S> HttpApplication<S> where S: 'static
{
    pub(crate) fn prepare(self, prefix: String) -> Box<HttpHandler> {
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

impl HttpApplication<()> {
    pub fn no_state() -> Self {
        HttpApplication {
            state: (),
            default: HttpResource::default(),
            resources: HashMap::new(),
        }
    }
}

impl<S> HttpApplication<S> where S: 'static {

    pub fn new(state: S) -> HttpApplication<S> {
        HttpApplication {
            state: state,
            default: HttpResource::default(),
            resources: HashMap::new(),
        }
    }

    pub fn add<P: ToString>(&mut self, path: P) -> &mut HttpResource<S>
    {
        let path = path.to_string();

        // add resource
        if !self.resources.contains_key(&path) {
            self.resources.insert(path.clone(), HttpResource::default());
        }

        self.resources.get_mut(&path).unwrap()
    }

    /// Default resource
    pub fn default(&mut self) -> &mut HttpResource<S> {
        &mut self.default
    }
}


pub(crate)
struct InnerApplication<S> {
    state: Rc<S>,
    default: HttpResource<S>,
    router: Router<HttpResource<S>>,
}


impl<S: 'static> HttpHandler for InnerApplication<S> {

    fn handle(&self, req: HttpRequest, payload: Option<Payload>) -> Task {
        if let Ok(h) = self.router.recognize(req.path()) {
            h.handler.handle(req.with_params(h.params), payload, Rc::clone(&self.state))
        } else {
            self.default.handle(req, payload, Rc::clone(&self.state))
        }
    }
}
