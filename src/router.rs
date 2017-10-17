use std::rc::Rc;
use std::string::ToString;
use std::collections::HashMap;

use task::Task;
use payload::Payload;
use route::RouteHandler;
use resource::Resource;
use recognizer::{RouteRecognizer, check_pattern};
use application::Application;
use httpcodes::HTTPNotFound;
use httprequest::HttpRequest;

pub(crate) trait Handler: 'static {
    fn handle(&self, req: HttpRequest, payload: Payload) -> Task;
}

/// Server routing map
pub struct Router {
    apps: HashMap<String, Box<Handler>>,
    resources: RouteRecognizer<Resource>,
}

impl Router {

    pub(crate) fn call(&self, req: HttpRequest, payload: Payload) -> Task
    {
        if let Some((params, h)) = self.resources.recognize(req.path()) {
            if let Some(params) = params {
                h.handle(
                    req.with_match_info(params), payload, Rc::new(()))
            } else {
                h.handle(req, payload, Rc::new(()))
            }
        } else {
            for (prefix, app) in &self.apps {
                if req.path().starts_with(prefix) {
                    return app.handle(req, payload)
                }
            }
            Task::reply(HTTPNotFound.response())
        }
    }
}

/// Request routing map builder
///
/// Resource may have variable path also. For instance, a resource with
/// the path */a/{name}/c* would match all incoming requests with paths
/// such as */a/b/c*, */a/1/c*, and */a/etc/c*.
///
/// A variable part is specified in the form `{identifier}`, where
/// the identifier can be used later in a request handler to access the matched
/// value for that part. This is done by looking up the identifier
/// in the Params object returned by `Request.match_info()` method.
///
/// By default, each part matches the regular expression `[^{}/]+`.
///
/// You can also specify a custom regex in the form `{identifier:regex}`:
///
/// For instance, to route Get requests on any route matching `/users/{userid}/{friend}` and
/// store userid and friend in the exposed Params object:
///
/// ```rust,ignore
/// let mut map = RoutingMap::default();
///
/// map.resource("/users/{userid}/{friend}", |r| r.get::<MyRoute>());
/// ```
pub struct RoutingMap {
    parts: Option<RoutingMapParts>,
}

struct RoutingMapParts {
    apps: HashMap<String, Box<Handler>>,
    resources: HashMap<String, Resource>,
}

impl Default for RoutingMap {
    fn default() -> Self {
        RoutingMap {
            parts: Some(RoutingMapParts {
                apps: HashMap::new(),
                resources: HashMap::new()}),
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
    ///     let mut router =
    ///         RoutingMap::default()
    ///             .app("/pre", Application::default()
    ///                  .resource("/users/{userid}", |r| {
    ///                      r.get::<MyRoute>();
    ///                      r.post::<MyRoute>();
    ///                 })
    ///                 .finish()
    ///         ).finish();
    /// }
    /// ```
    /// In this example, `MyRoute` route is available as `http://.../pre/test` url.
    pub fn app<P, S: 'static>(&mut self, prefix: P, app: Application<S>) -> &mut Self
        where P: ToString
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // we can not override registered resource
            let prefix = prefix.to_string();
            if parts.apps.contains_key(&prefix) {
                panic!("Resource is registered: {}", prefix);
            }

            // add application
            parts.apps.insert(prefix.clone(), app.prepare(prefix));
        }
        self
    }

    /// Configure resource for specific path.
    ///
    /// ```rust,ignore
    ///
    /// struct MyRoute;
    ///
    /// fn main() {
    ///     RoutingMap::default().resource("/test", |r| {
    ///         r.post::<MyRoute>();
    ///     }).finish();
    /// }
    /// ```
    /// In this example, `MyRoute` route is available as `http://.../test` url.
    pub fn resource<P, F>(&mut self, path: P, f: F) -> &mut Self
        where F: FnOnce(&mut Resource<()>) + 'static,
              P: ToString,
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // add resource
            let path = path.to_string();
            if !parts.resources.contains_key(&path) {
                check_pattern(&path);
                parts.resources.insert(path.clone(), Resource::default());
            }
            // configure resource
            f(parts.resources.get_mut(&path).unwrap());
        }
        self
    }

    /// Finish configuration and create `Router` instance
    pub fn finish(&mut self) -> Router
    {
        let parts = self.parts.take().expect("Use after finish");

        let mut routes = Vec::new();
        for (path, resource) in parts.resources {
            routes.push((path, resource))
        }

        Router {
            apps: parts.apps,
            resources: RouteRecognizer::new("/".to_owned(), routes),
        }
    }
}
