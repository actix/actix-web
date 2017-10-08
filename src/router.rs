use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;
use route_recognizer::{Router as Recognizer};

use task::Task;
use route::{Payload, RouteHandler};
use resource::Resource;
use application::Application;
use httpcodes::HTTPNotFound;
use httpmessage::{HttpRequest, IntoHttpResponse};

pub(crate) trait Handler: 'static {
    fn handle(&self, req: HttpRequest, payload: Option<Payload>) -> Task;
}

/// Request routing map
///
/// Route supports glob patterns: * for a single wildcard segment and :param
/// for matching storing that segment of the request url in the Params object,
/// which is stored in the request.
///
/// For instance, to route Get requests on any route matching /users/:userid/:friend and
/// store userid and friend in the exposed Params object:
///
/// ```rust,ignore
/// let mut router = RoutingMap::default();
///
/// router.add_resource("/users/:userid/:friendid").get::<MyRoute>();
/// ```
pub struct RoutingMap {
    apps: HashMap<String, Box<Handler>>,
    resources: HashMap<String, Resource>,
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

    /// Add `Application` object with specific prefix.
    /// Application prefixes all registered resources with specified prefix.
    ///
    /// ```rust,ignore
    ///
    /// struct MyRoute;
    ///
    /// fn main() {
    ///     let mut app = Application::default();
    ///     app.add("/test")
    ///         .get::<MyRoute>()
    ///         .post::<MyRoute>();
    ///
    ///     let mut routes = RoutingMap::default();
    ///     routes.add("/pre", app);
    /// }
    /// ```
    /// In this example, `MyRoute` route is available as `http://.../pre/test` url.
    pub fn add<P, S: 'static>(&mut self, prefix: P, app: Application<S>)
        where P: ToString
    {
        let prefix = prefix.to_string();

        // we can not override registered resource
        if self.apps.contains_key(&prefix) {
            panic!("Resource is registered: {}", prefix);
        }

        // add application
        self.apps.insert(prefix.clone(), app.prepare(prefix));
    }

    /// This method creates `Resource` for specified path
    /// or returns mutable reference to resource object.
    ///
    /// ```rust,ignore
    ///
    /// struct MyRoute;
    ///
    /// fn main() {
    ///     let mut routes = RoutingMap::default();
    ///
    ///     routes.add_resource("/test")
    ///         .post::<MyRoute>();
    /// }
    /// ```
    /// In this example, `MyRoute` route is available as `http://.../test` url.
    pub fn add_resource<P>(&mut self, path: P) -> &mut Resource
        where P: ToString
    {
        let path = path.to_string();

        // add resource
        if !self.resources.contains_key(&path) {
            self.resources.insert(path.clone(), Resource::default());
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
    apps: HashMap<String, Box<Handler>>,
    resources: Recognizer<Resource>,
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

            Task::reply(IntoHttpResponse::response(HTTPNotFound, req))
        }
    }
}
