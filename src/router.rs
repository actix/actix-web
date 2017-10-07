use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;
use route_recognizer::{Router as Recognizer};

use task::Task;
use route::{Payload, RouteHandler};
use resource::HttpResource;
use application::HttpApplication;
use httpcodes::HTTPNotFound;
use httpmessage::{HttpRequest, IntoHttpResponse};

pub trait HttpHandler: 'static {
    fn handle(&self, req: HttpRequest, payload: Option<Payload>) -> Task;
}

pub struct RoutingMap {
    apps: HashMap<String, Box<HttpHandler>>,
    resources: HashMap<String, HttpResource>,
}

impl Default for RoutingMap {
    fn default() -> Self {
        RoutingMap {
            apps: HashMap::new(),
            resources: HashMap::new()
        }
    }
}

impl RoutingMap {

    pub fn add<P, S: 'static>(&mut self, path: P, app: HttpApplication<S>)
        where P: ToString
    {
        let path = path.to_string();

        // we can not override registered resource
        if self.apps.contains_key(&path) {
            panic!("Resource is registered: {}", path);
        }

        // add application
        self.apps.insert(path.clone(), app.prepare(path));
    }

    pub fn add_resource<P>(&mut self, path: P) -> &mut HttpResource
        where P: ToString
    {
        let path = path.to_string();

        // add resource
        if !self.resources.contains_key(&path) {
            self.resources.insert(path.clone(), HttpResource::default());
        }

        self.resources.get_mut(&path).unwrap()
    }

    pub(crate) fn into_router(self) -> Router {
        let mut router = Recognizer::new();

        for (path, resource) in self.resources {
            router.add(path.as_str(), resource);
        }

        Router {
            apps: self.apps,
            resources: router,
        }
    }
}


pub(crate)
struct Router {
    apps: HashMap<String, Box<HttpHandler>>,
    resources: Recognizer<HttpResource>,
}

impl Router {

    pub fn call(&self, req: HttpRequest, payload: Option<Payload>) -> Task
    {
        if let Ok(h) = self.resources.recognize(req.path()) {
            h.handler.handle(req.with_params(h.params), payload, Rc::new(()))
        } else {
            for (prefix, app) in &self.apps {
                if req.path().starts_with(prefix) {
                    return app.handle(req, payload)
                }
            }

            Task::reply(IntoHttpResponse::into_response(HTTPNotFound, req), None)
        }
    }
}
