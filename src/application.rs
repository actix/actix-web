use std::mem;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;

use handler::Reply;
use router::{Router, Pattern};
use resource::Resource;
use header::ContentEncoding;
use handler::{Handler, RouteHandler, WrapHandler};
use httprequest::HttpRequest;
use pipeline::{Pipeline, PipelineHandler};
use middleware::Middleware;
use server::{HttpHandler, IntoHttpHandler, HttpHandlerTask, ServerSettings};

/// Application
pub struct HttpApplication<S=()> {
    state: Rc<S>,
    prefix: String,
    router: Router,
    inner: Rc<RefCell<Inner<S>>>,
    middlewares: Rc<Vec<Box<Middleware<S>>>>,
}

pub(crate) struct Inner<S> {
    prefix: usize,
    default: Resource<S>,
    encoding: ContentEncoding,
    router: Router,
    resources: Vec<Resource<S>>,
    handlers: Vec<(String, Box<RouteHandler<S>>)>,
}

impl<S: 'static> PipelineHandler<S> for Inner<S> {

    fn encoding(&self) -> ContentEncoding {
        self.encoding
    }

    fn handle(&mut self, mut req: HttpRequest<S>) -> Reply {
        if let Some(idx) = self.router.recognize(&mut req) {
            self.resources[idx].handle(req.clone(), Some(&mut self.default))
        } else {
            for &mut (ref prefix, ref mut handler) in &mut self.handlers {
                let m = {
                    let path = &req.path()[self.prefix..];
                    path.starts_with(prefix) && (
                        path.len() == prefix.len() ||
                            path.split_at(prefix.len()).1.starts_with('/'))
                };
                if m {
                    let path: &'static str = unsafe {
                        mem::transmute(&req.path()[self.prefix+prefix.len()..]) };
                    if path.is_empty() {
                        req.match_info_mut().add("tail", "");
                    } else {
                        req.match_info_mut().add("tail", path.split_at(1).1);
                    }
                    return handler.handle(req)
                }
            }
            self.default.handle(req, None)
        }
    }
}

#[cfg(test)]
impl<S: 'static> HttpApplication<S> {
    #[cfg(test)]
    pub(crate) fn run(&mut self, req: HttpRequest<S>) -> Reply {
        self.inner.borrow_mut().handle(req)
    }
    #[cfg(test)]
    pub(crate) fn prepare_request(&self, req: HttpRequest) -> HttpRequest<S> {
        req.with_state(Rc::clone(&self.state), self.router.clone())
    }
}

impl<S: 'static> HttpHandler for HttpApplication<S> {

    fn handle(&mut self, req: HttpRequest) -> Result<Box<HttpHandlerTask>, HttpRequest> {
        let m = {
            let path = req.path();
            path.starts_with(&self.prefix) && (
                path.len() == self.prefix.len() ||
                    path.split_at(self.prefix.len()).1.starts_with('/'))
        };
        if m {
            let inner = Rc::clone(&self.inner);
            let req = req.with_state(Rc::clone(&self.state), self.router.clone());

            Ok(Box::new(Pipeline::new(req, Rc::clone(&self.middlewares), inner)))
        } else {
            Err(req)
        }
    }
}

struct ApplicationParts<S> {
    state: S,
    prefix: String,
    settings: ServerSettings,
    default: Resource<S>,
    resources: Vec<(Pattern, Option<Resource<S>>)>,
    handlers: Vec<(String, Box<RouteHandler<S>>)>,
    external: HashMap<String, Pattern>,
    encoding: ContentEncoding,
    middlewares: Vec<Box<Middleware<S>>>,
}

/// Structure that follows the builder pattern for building `Application` structs.
pub struct Application<S=()> {
    parts: Option<ApplicationParts<S>>,
}

impl Application<()> {

    /// Create application with empty state. Application can
    /// be configured with builder-like pattern.
    pub fn new() -> Application<()> {
        Application {
            parts: Some(ApplicationParts {
                state: (),
                prefix: "/".to_owned(),
                settings: ServerSettings::default(),
                default: Resource::default_not_found(),
                resources: Vec::new(),
                handlers: Vec::new(),
                external: HashMap::new(),
                encoding: ContentEncoding::Auto,
                middlewares: Vec::new(),
            })
        }
    }
}

impl Default for Application<()> {
    fn default() -> Self {
        Application::new()
    }
}

impl<S> Application<S> where S: 'static {

    /// Create application with specific state. Application can be
    /// configured with builder-like pattern.
    ///
    /// State is shared with all resources within same application and could be
    /// accessed with `HttpRequest::state()` method.
    pub fn with_state(state: S) -> Application<S> {
        Application {
            parts: Some(ApplicationParts {
                state,
                prefix: "/".to_owned(),
                settings: ServerSettings::default(),
                default: Resource::default_not_found(),
                resources: Vec::new(),
                handlers: Vec::new(),
                external: HashMap::new(),
                middlewares: Vec::new(),
                encoding: ContentEncoding::Auto,
            })
        }
    }

    /// Set application prefix
    ///
    /// Only requests that matches application's prefix get processed by this application.
    /// Application prefix always contains leading "/" slash. If supplied prefix
    /// does not contain leading slash, it get inserted. Prefix should
    /// consists valid path segments. i.e for application with
    /// prefix `/app` any request with following paths `/app`, `/app/` or `/app/test`
    /// would match, but path `/application` would not match.
    ///
    /// In the following example only requests with "/app/" path prefix
    /// get handled. Request with path "/app/test/" would be handled,
    /// but request with path "/application" or "/other/..." would return *NOT FOUND*
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, Application, HttpResponse};
    ///
    /// fn main() {
    ///     let app = Application::new()
    ///         .prefix("/app")
    ///         .resource("/test", |r| {
    ///              r.method(http::Method::GET).f(|_| HttpResponse::Ok());
    ///              r.method(http::Method::HEAD).f(|_| HttpResponse::MethodNotAllowed());
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn prefix<P: Into<String>>(mut self, prefix: P) -> Application<S> {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            let mut prefix = prefix.into();
            if !prefix.starts_with('/') {
                prefix.insert(0, '/')
            }
            parts.prefix = prefix;
        }
        self
    }

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
    /// # extern crate actix_web;
    /// use actix_web::{http, Application, HttpResponse};
    ///
    /// fn main() {
    ///     let app = Application::new()
    ///         .resource("/test", |r| {
    ///              r.method(http::Method::GET).f(|_| HttpResponse::Ok());
    ///              r.method(http::Method::HEAD).f(|_| HttpResponse::MethodNotAllowed());
    ///         });
    /// }
    /// ```
    pub fn resource<F>(mut self, path: &str, f: F) -> Application<S>
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // add resource
            let mut resource = Resource::default();
            f(&mut resource);

            let pattern = Pattern::new(resource.get_name(), path);
            parts.resources.push((pattern, Some(resource)));
        }
        self
    }

    /// Default resource is used if no matched route could be found.
    pub fn default_resource<F>(mut self, f: F) -> Application<S>
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            f(&mut parts.default);
        }
        self
    }

    /// Set default content encoding. `ContentEncoding::Auto` is set by default.
    pub fn default_encoding<F>(mut self, encoding: ContentEncoding) -> Application<S>
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            parts.encoding = encoding;
        }
        self
    }

    /// Register external resource.
    ///
    /// External resources are useful for URL generation purposes only and
    /// are never considered for matching at request time.
    /// Call to `HttpRequest::url_for()` will work as expected.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{Application, HttpRequest, HttpResponse, Result};
    ///
    /// fn index(mut req: HttpRequest) -> Result<HttpResponse> {
    ///    let url = req.url_for("youtube", &["oHg5SJYRHA0"])?;
    ///    assert_eq!(url.as_str(), "https://youtube.com/watch/oHg5SJYRHA0");
    ///    Ok(HttpResponse::Ok().into())
    /// }
    ///
    /// fn main() {
    ///     let app = Application::new()
    ///         .resource("/index.html", |r| r.f(index))
    ///         .external_resource("youtube", "https://youtube.com/watch/{video_id}")
    ///         .finish();
    /// }
    /// ```
    pub fn external_resource<T, U>(mut self, name: T, url: U) -> Application<S>
        where T: AsRef<str>, U: AsRef<str>
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            if parts.external.contains_key(name.as_ref()) {
                panic!("External resource {:?} is registered.", name.as_ref());
            }
            parts.external.insert(
                String::from(name.as_ref()), Pattern::new(name.as_ref(), url.as_ref()));
        }
        self
    }

    /// Configure handler for specific path prefix.
    ///
    /// Path prefix consists valid path segments. i.e for prefix `/app`
    /// any request with following paths `/app`, `/app/` or `/app/test`
    /// would match, but path `/application` would not match.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, Application, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = Application::new()
    ///         .handler("/app", |req: HttpRequest| {
    ///             match *req.method() {
    ///                 http::Method::GET => HttpResponse::Ok(),
    ///                 http::Method::POST => HttpResponse::MethodNotAllowed(),
    ///                 _ => HttpResponse::NotFound(),
    ///         }});
    /// }
    /// ```
    pub fn handler<H: Handler<S>>(mut self, path: &str, handler: H) -> Application<S>
    {
        {
            let path = path.trim().trim_right_matches('/').to_owned();
            let parts = self.parts.as_mut().expect("Use after finish");
            parts.handlers.push((path, Box::new(WrapHandler::new(handler))));
        }
        self
    }

    /// Register a middleware
    pub fn middleware<M: Middleware<S>>(mut self, mw: M) -> Application<S> {
        self.parts.as_mut().expect("Use after finish")
            .middlewares.push(Box::new(mw));
        self
    }

    /// Run external configuration as part of application building process
    ///
    /// This function is useful for moving part of configuration to a different
    /// module or event library. For example we can move some of the resources
    /// configuration to different module.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{Application, HttpResponse, http, fs, middleware};
    ///
    /// // this function could be located in different module
    /// fn config(app: Application) -> Application {
    ///     app
    ///         .resource("/test", |r| {
    ///              r.method(http::Method::GET).f(|_| HttpResponse::Ok());
    ///              r.method(http::Method::HEAD).f(|_| HttpResponse::MethodNotAllowed());
    ///         })
    /// }
    ///
    /// fn main() {
    ///     let app = Application::new()
    ///         .middleware(middleware::Logger::default())
    ///         .configure(config)  // <- register resources
    ///         .handler("/static", fs::StaticFiles::new(".", true));
    /// }
    /// ```
    pub fn configure<F>(self, cfg: F) -> Application<S>
        where F: Fn(Application<S>) -> Application<S>
    {
        cfg(self)
    }

    /// Finish application configuration and create HttpHandler object
    pub fn finish(&mut self) -> HttpApplication<S> {
        let parts = self.parts.take().expect("Use after finish");
        let prefix = parts.prefix.trim().trim_right_matches('/');

        let mut resources = parts.resources;
        for (_, pattern) in parts.external {
            resources.push((pattern, None));
        }

        let (router, resources) = Router::new(prefix, parts.settings, resources);

        let inner = Rc::new(RefCell::new(
            Inner {
                prefix: prefix.len(),
                default: parts.default,
                encoding: parts.encoding,
                router: router.clone(),
                handlers: parts.handlers,
                resources,
            }
        ));

        HttpApplication {
            state: Rc::new(parts.state),
            prefix: prefix.to_owned(),
            router: router.clone(),
            middlewares: Rc::new(parts.middlewares),
            inner,
        }
    }

    /// Convenience method for creating `Box<HttpHandler>` instance.
    ///
    /// This method is useful if you need to register several application instances
    /// with different state.
    ///
    /// ```rust
    /// # use std::thread;
    /// # extern crate actix_web;
    /// use actix_web::*;
    ///
    /// struct State1;
    ///
    /// struct State2;
    ///
    /// fn main() {
    /// # thread::spawn(|| {
    ///     HttpServer::new(|| { vec![
    ///         Application::with_state(State1)
    ///              .prefix("/app1")
    ///              .resource("/", |r| r.f(|r| HttpResponse::Ok()))
    ///              .boxed(),
    ///         Application::with_state(State2)
    ///              .prefix("/app2")
    ///              .resource("/", |r| r.f(|r| HttpResponse::Ok()))
    ///              .boxed() ]})
    ///         .bind("127.0.0.1:8080").unwrap()
    ///         .run()
    /// # });
    /// }
    /// ```
    pub fn boxed(mut self) -> Box<HttpHandler> {
        Box::new(self.finish())
    }
}

impl<S: 'static> IntoHttpHandler for Application<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(mut self, settings: ServerSettings) -> HttpApplication<S> {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            parts.settings = settings;
        }
        self.finish()
    }
}

impl<'a, S: 'static> IntoHttpHandler for &'a mut Application<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(self, settings: ServerSettings) -> HttpApplication<S> {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            parts.settings = settings;
        }
        self.finish()
    }
}

#[doc(hidden)]
impl<S: 'static> Iterator for Application<S> {
    type Item = HttpApplication<S>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.parts.is_some() {
            Some(self.finish())
        } else {
            None
        }
    }
}


#[cfg(test)]
mod tests {
    use http::StatusCode;
    use super::*;
    use test::TestRequest;
    use httprequest::HttpRequest;
    use httpresponse::HttpResponse;

    #[test]
    fn test_default_resource() {
        let mut app = Application::new()
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .finish();

        let req = TestRequest::with_uri("/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::NOT_FOUND);

        let mut app = Application::new()
            .default_resource(|r| r.f(|_| HttpResponse::MethodNotAllowed()))
            .finish();
        let req = TestRequest::with_uri("/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_unhandled_prefix() {
        let mut app = Application::new()
            .prefix("/test")
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        assert!(app.handle(HttpRequest::default()).is_err());
    }

    #[test]
    fn test_state() {
        let mut app = Application::with_state(10)
            .resource("/", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        let req = HttpRequest::default().with_state(Rc::clone(&app.state), app.router.clone());
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);
    }

    #[test]
    fn test_prefix() {
        let mut app = Application::new()
            .prefix("/test")
            .resource("/blah", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        let req = TestRequest::with_uri("/test").finish();
        let resp = app.handle(req);
        assert!(resp.is_ok());

        let req = TestRequest::with_uri("/test/").finish();
        let resp = app.handle(req);
        assert!(resp.is_ok());

        let req = TestRequest::with_uri("/test/blah").finish();
        let resp = app.handle(req);
        assert!(resp.is_ok());

        let req = TestRequest::with_uri("/testing").finish();
        let resp = app.handle(req);
        assert!(resp.is_err());
    }

    #[test]
    fn test_handler() {
        let mut app = Application::new()
            .handler("/test", |_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/app").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/testapp").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handler_prefix() {
        let mut app = Application::new()
            .prefix("/app")
            .handler("/test", |_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/test/").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/test/app").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/testapp").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::NOT_FOUND);

    }

}
