use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::rc::Rc;

use handler::{AsyncResult, FromRequest, Handler, Responder, RouteHandler, WrapHandler};
use header::ContentEncoding;
use http::{Method, StatusCode};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::Middleware;
use pipeline::{HandlerType, Pipeline, PipelineHandler};
use pred::Predicate;
use resource::ResourceHandler;
use router::{Resource, Router};
use scope::Scope;
use server::{HttpHandler, HttpHandlerTask, IntoHttpHandler, ServerSettings};

/// Application
pub struct HttpApplication<S = ()> {
    state: Rc<S>,
    prefix: String,
    prefix_len: usize,
    router: Router,
    inner: Rc<UnsafeCell<Inner<S>>>,
    filters: Option<Vec<Box<Predicate<S>>>>,
    middlewares: Rc<RefCell<Vec<Box<Middleware<S>>>>>,
}

#[doc(hidden)]
pub struct Inner<S> {
    prefix: usize,
    default: ResourceHandler<S>,
    encoding: ContentEncoding,
    resources: Vec<ResourceHandler<S>>,
    handlers: Vec<PrefixHandlerType<S>>,
}

enum PrefixHandlerType<S> {
    Handler(String, Box<RouteHandler<S>>),
    Scope(Resource, Box<RouteHandler<S>>, Vec<Box<Predicate<S>>>),
}

impl<S: 'static> PipelineHandler<S> for Inner<S> {
    fn encoding(&self) -> ContentEncoding {
        self.encoding
    }

    fn handle(
        &mut self, req: HttpRequest<S>, htype: HandlerType,
    ) -> AsyncResult<HttpResponse> {
        match htype {
            HandlerType::Normal(idx) => match self.resources[idx].handle(req) {
                Ok(result) => result,
                Err(req) => match self.default.handle(req) {
                    Ok(result) => result,
                    Err(_) => AsyncResult::ok(HttpResponse::new(StatusCode::NOT_FOUND)),
                },
            },
            HandlerType::Handler(idx) => match self.handlers[idx] {
                PrefixHandlerType::Handler(_, ref mut hnd) => hnd.handle(req),
                PrefixHandlerType::Scope(_, ref mut hnd, _) => hnd.handle(req),
            },
            HandlerType::Default => match self.default.handle(req) {
                Ok(result) => result,
                Err(_) => AsyncResult::ok(HttpResponse::new(StatusCode::NOT_FOUND)),
            },
        }
    }
}

impl<S: 'static> HttpApplication<S> {
    #[inline]
    fn as_ref(&self) -> &Inner<S> {
        unsafe { &*self.inner.get() }
    }

    #[inline]
    fn get_handler(&self, req: &mut HttpRequest<S>) -> HandlerType {
        if let Some(idx) = self.router.recognize(req) {
            HandlerType::Normal(idx)
        } else {
            let inner = self.as_ref();
            req.match_info_mut().set_tail(0);

            'outer: for idx in 0..inner.handlers.len() {
                match inner.handlers[idx] {
                    PrefixHandlerType::Handler(ref prefix, _) => {
                        let m = {
                            let path = &req.path()[inner.prefix..];
                            let path_len = path.len();

                            path.starts_with(prefix)
                                && (path_len == prefix.len()
                                    || path.split_at(prefix.len()).1.starts_with('/'))
                        };

                        if m {
                            let prefix_len = (inner.prefix + prefix.len()) as u16;
                            let url = req.url().clone();
                            req.set_prefix_len(prefix_len);
                            req.match_info_mut().set_url(url);
                            req.match_info_mut().set_tail(prefix_len);
                            return HandlerType::Handler(idx);
                        }
                    }
                    PrefixHandlerType::Scope(ref pattern, _, ref filters) => {
                        if let Some(prefix_len) =
                            pattern.match_prefix_with_params(req, inner.prefix)
                        {
                            for filter in filters {
                                if !filter.check(req) {
                                    continue 'outer;
                                }
                            }

                            let prefix_len = (inner.prefix + prefix_len) as u16;
                            let url = req.url().clone();
                            req.set_prefix_len(prefix_len);
                            let params = req.match_info_mut();
                            params.set_tail(prefix_len);
                            params.set_url(url);
                            return HandlerType::Handler(idx);
                        }
                    }
                }
            }
            HandlerType::Default
        }
    }

    #[cfg(test)]
    pub(crate) fn run(&mut self, mut req: HttpRequest<S>) -> AsyncResult<HttpResponse> {
        let tp = self.get_handler(&mut req);
        unsafe { &mut *self.inner.get() }.handle(req, tp)
    }

    #[cfg(test)]
    pub(crate) fn prepare_request(&self, req: HttpRequest) -> HttpRequest<S> {
        req.with_state(Rc::clone(&self.state), self.router.clone())
    }
}

impl<S: 'static> HttpHandler for HttpApplication<S> {
    type Task = Pipeline<S, Inner<S>>;

    fn handle(
        &mut self, req: HttpRequest,
    ) -> Result<Pipeline<S, Inner<S>>, HttpRequest> {
        let m = {
            let path = req.path();
            path.starts_with(&self.prefix)
                && (path.len() == self.prefix_len
                    || path.split_at(self.prefix_len).1.starts_with('/'))
        };
        if m {
            let mut req2 =
                req.clone_with_state(Rc::clone(&self.state), self.router.clone());

            if let Some(ref filters) = self.filters {
                for filter in filters {
                    if !filter.check(&mut req2) {
                        return Err(req);
                    }
                }
            }

            let tp = self.get_handler(&mut req2);
            let inner = Rc::clone(&self.inner);
            Ok(Pipeline::new(req2, Rc::clone(&self.middlewares), inner, tp))
        } else {
            Err(req)
        }
    }
}

struct ApplicationParts<S> {
    state: S,
    prefix: String,
    settings: ServerSettings,
    default: ResourceHandler<S>,
    resources: Vec<(Resource, Option<ResourceHandler<S>>)>,
    handlers: Vec<PrefixHandlerType<S>>,
    external: HashMap<String, Resource>,
    encoding: ContentEncoding,
    middlewares: Vec<Box<Middleware<S>>>,
    filters: Vec<Box<Predicate<S>>>,
}

/// Structure that follows the builder pattern for building application
/// instances.
pub struct App<S = ()> {
    parts: Option<ApplicationParts<S>>,
}

impl App<()> {
    /// Create application with empty state. Application can
    /// be configured with a builder-like pattern.
    pub fn new() -> App<()> {
        App::with_state(())
    }
}

impl Default for App<()> {
    fn default() -> Self {
        App::new()
    }
}

impl<S> App<S>
where
    S: 'static,
{
    /// Create application with specified state. Application can be
    /// configured with a builder-like pattern.
    ///
    /// State is shared with all resources within same application and
    /// could be accessed with `HttpRequest::state()` method.
    ///
    /// **Note**: http server accepts an application factory rather than
    /// an application instance. Http server constructs an application
    /// instance for each thread, thus application state must be constructed
    /// multiple times. If you want to share state between different
    /// threads, a shared object should be used, e.g. `Arc`. Application
    /// state does not need to be `Send` and `Sync`.
    pub fn with_state(state: S) -> App<S> {
        App {
            parts: Some(ApplicationParts {
                state,
                prefix: "/".to_owned(),
                settings: ServerSettings::default(),
                default: ResourceHandler::default_not_found(),
                resources: Vec::new(),
                handlers: Vec::new(),
                external: HashMap::new(),
                middlewares: Vec::new(),
                filters: Vec::new(),
                encoding: ContentEncoding::Auto,
            }),
        }
    }

    /// Get reference to the application state
    pub fn state(&self) -> &S {
        let parts = self.parts.as_ref().expect("Use after finish");
        &parts.state
    }

    /// Set application prefix.
    ///
    /// Only requests that match the application's prefix get
    /// processed by this application.
    ///
    /// The application prefix always contains a leading slash (`/`).
    /// If the supplied prefix does not contain leading slash, it is
    /// inserted.
    ///
    /// Prefix should consist of valid path segments. i.e for an
    /// application with the prefix `/app` any request with the paths
    /// `/app`, `/app/` or `/app/test` would match, but the path
    /// `/application` would not.
    ///
    /// In the following example only requests with an `/app/` path
    /// prefix get handled. Requests with path `/app/test/` would be
    /// handled, while requests with the paths `/application` or
    /// `/other/...` would return `NOT FOUND`.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .prefix("/app")
    ///         .resource("/test", |r| {
    ///             r.get().f(|_| HttpResponse::Ok());
    ///             r.head().f(|_| HttpResponse::MethodNotAllowed());
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn prefix<P: Into<String>>(mut self, prefix: P) -> App<S> {
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

    /// Add match predicate to application.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn main() {
    /// App::new()
    ///     .filter(pred::Host("www.rust-lang.org"))
    ///     .resource("/path", |r| r.f(|_| HttpResponse::Ok()))
    /// #      .finish();
    /// # }
    /// ```
    pub fn filter<T: Predicate<S> + 'static>(mut self, p: T) -> App<S> {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            parts.filters.push(Box::new(p));
        }
        self
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::resource()` method.
    /// Handler functions need to accept one request extractor
    /// argument.
    ///
    /// This method could be called multiple times, in that case
    /// multiple routes would be registered for same resource path.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .route("/test", http::Method::GET, |_: HttpRequest| {
    ///             HttpResponse::Ok()
    ///         })
    ///         .route("/test", http::Method::POST, |_: HttpRequest| {
    ///             HttpResponse::MethodNotAllowed()
    ///         });
    /// }
    /// ```
    pub fn route<T, F, R>(mut self, path: &str, method: Method, f: F) -> App<S>
    where
        F: Fn(T) -> R + 'static,
        R: Responder + 'static,
        T: FromRequest<S> + 'static,
    {
        {
            let parts: &mut ApplicationParts<S> = unsafe {
                &mut *(self.parts.as_mut().expect("Use after finish") as *mut _)
            };

            // get resource handler
            for &mut (ref pattern, ref mut handler) in &mut parts.resources {
                if let Some(ref mut handler) = *handler {
                    if pattern.pattern() == path {
                        handler.method(method).with(f);
                        return self;
                    }
                }
            }

            let mut handler = ResourceHandler::default();
            handler.method(method).with(f);
            let pattern = Resource::new(handler.get_name(), path);
            parts.resources.push((pattern, Some(handler)));
        }
        self
    }

    /// Configure scope for common root path.
    ///
    /// Scopes collect multiple paths under a common path prefix.
    /// Scope path can contain variable path segments as resources.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().scope("/{project_id}", |scope| {
    ///         scope
    ///             .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
    ///             .resource("/path2", |r| r.f(|_| HttpResponse::Ok()))
    ///             .resource("/path3", |r| r.f(|_| HttpResponse::MethodNotAllowed()))
    ///     });
    /// }
    /// ```
    ///
    /// In the above example, three routes get added:
    ///  * /{project_id}/path1
    ///  * /{project_id}/path2
    ///  * /{project_id}/path3
    ///
    pub fn scope<F>(mut self, path: &str, f: F) -> App<S>
    where
        F: FnOnce(Scope<S>) -> Scope<S>,
    {
        {
            let mut scope = Box::new(f(Scope::new()));
            let parts = self.parts.as_mut().expect("Use after finish");

            let filters = scope.take_filters();
            parts.handlers.push(PrefixHandlerType::Scope(
                Resource::prefix("", &path),
                scope,
                filters,
            ));
        }
        self
    }

    /// Configure resource for a specific path.
    ///
    /// Resources may have variable path segments. For example, a
    /// resource with the path `/a/{name}/c` would match all incoming
    /// requests with paths such as `/a/b/c`, `/a/1/c`, or `/a/etc/c`.
    ///
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment. This is done by
    /// looking up the identifier in the `Params` object returned by
    /// `HttpRequest.match_info()` method.
    ///
    /// By default, each segment matches the regular expression `[^{}/]+`.
    ///
    /// You can also specify a custom regex in the form `{identifier:regex}`:
    ///
    /// For instance, to route `GET`-requests on any route matching
    /// `/users/{userid}/{friend}` and store `userid` and `friend` in
    /// the exposed `Params` object:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().resource("/users/{userid}/{friend}", |r| {
    ///         r.get().f(|_| HttpResponse::Ok());
    ///         r.head().f(|_| HttpResponse::MethodNotAllowed());
    ///     });
    /// }
    /// ```
    pub fn resource<F, R>(mut self, path: &str, f: F) -> App<S>
    where
        F: FnOnce(&mut ResourceHandler<S>) -> R + 'static,
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // add resource handler
            let mut handler = ResourceHandler::default();
            f(&mut handler);

            let pattern = Resource::new(handler.get_name(), path);
            parts.resources.push((pattern, Some(handler)));
        }
        self
    }

    /// Configure resource for a specific path.
    #[doc(hidden)]
    pub fn register_resource(&mut self, path: &str, resource: ResourceHandler<S>) {
        let pattern = Resource::new(resource.get_name(), path);
        self.parts
            .as_mut()
            .expect("Use after finish")
            .resources
            .push((pattern, Some(resource)));
    }

    /// Default resource to be used if no matching route could be found.
    pub fn default_resource<F, R>(mut self, f: F) -> App<S>
    where
        F: FnOnce(&mut ResourceHandler<S>) -> R + 'static,
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            f(&mut parts.default);
        }
        self
    }

    /// Set default content encoding. `ContentEncoding::Auto` is set by default.
    pub fn default_encoding(mut self, encoding: ContentEncoding) -> App<S> {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            parts.encoding = encoding;
        }
        self
    }

    /// Register an external resource.
    ///
    /// External resources are useful for URL generation purposes only
    /// and are never considered for matching at request time. Calls to
    /// `HttpRequest::url_for()` will work as expected.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{App, HttpRequest, HttpResponse, Result};
    ///
    /// fn index(mut req: HttpRequest) -> Result<HttpResponse> {
    ///     let url = req.url_for("youtube", &["oHg5SJYRHA0"])?;
    ///     assert_eq!(url.as_str(), "https://youtube.com/watch/oHg5SJYRHA0");
    ///     Ok(HttpResponse::Ok().into())
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .resource("/index.html", |r| r.get().f(index))
    ///         .external_resource("youtube", "https://youtube.com/watch/{video_id}")
    ///         .finish();
    /// }
    /// ```
    pub fn external_resource<T, U>(mut self, name: T, url: U) -> App<S>
    where
        T: AsRef<str>,
        U: AsRef<str>,
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            if parts.external.contains_key(name.as_ref()) {
                panic!("External resource {:?} is registered.", name.as_ref());
            }
            parts.external.insert(
                String::from(name.as_ref()),
                Resource::external(name.as_ref(), url.as_ref()),
            );
        }
        self
    }

    /// Configure handler for specific path prefix.
    ///
    /// A path prefix consists of valid path segments, i.e for the
    /// prefix `/app` any request with the paths `/app`, `/app/` or
    /// `/app/test` would match, but the path `/application` would
    /// not.
    ///
    /// Path tail is available as `tail` parameter in request's match_dict.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().handler("/app", |req: HttpRequest| match *req.method() {
    ///         http::Method::GET => HttpResponse::Ok(),
    ///         http::Method::POST => HttpResponse::MethodNotAllowed(),
    ///         _ => HttpResponse::NotFound(),
    ///     });
    /// }
    /// ```
    pub fn handler<H: Handler<S>>(mut self, path: &str, handler: H) -> App<S> {
        {
            let mut path = path.trim().trim_right_matches('/').to_owned();
            if !path.is_empty() && !path.starts_with('/') {
                path.insert(0, '/')
            }
            if path.len() > 1 && path.ends_with('/') {
                path.pop();
            }
            let parts = self.parts.as_mut().expect("Use after finish");

            parts.handlers.push(PrefixHandlerType::Handler(
                path,
                Box::new(WrapHandler::new(handler)),
            ));
        }
        self
    }

    /// Register a middleware.
    pub fn middleware<M: Middleware<S>>(mut self, mw: M) -> App<S> {
        self.parts
            .as_mut()
            .expect("Use after finish")
            .middlewares
            .push(Box::new(mw));
        self
    }

    /// Run external configuration as part of the application building
    /// process
    ///
    /// This function is useful for moving parts of configuration to a
    /// different module or event library. For example we can move
    /// some of the resources' configuration to different module.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{fs, middleware, App, HttpResponse};
    ///
    /// // this function could be located in different module
    /// fn config(app: App) -> App {
    ///     app.resource("/test", |r| {
    ///         r.get().f(|_| HttpResponse::Ok());
    ///         r.head().f(|_| HttpResponse::MethodNotAllowed());
    ///     })
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .middleware(middleware::Logger::default())
    ///         .configure(config)  // <- register resources
    ///         .handler("/static", fs::StaticFiles::new("."));
    /// }
    /// ```
    pub fn configure<F>(self, cfg: F) -> App<S>
    where
        F: Fn(App<S>) -> App<S>,
    {
        cfg(self)
    }

    /// Finish application configuration and create `HttpHandler` object.
    pub fn finish(&mut self) -> HttpApplication<S> {
        let parts = self.parts.take().expect("Use after finish");
        let prefix = parts.prefix.trim().trim_right_matches('/');
        let (prefix, prefix_len) = if prefix.is_empty() {
            ("/".to_owned(), 0)
        } else {
            (prefix.to_owned(), prefix.len())
        };

        let mut resources = parts.resources;
        for (_, pattern) in parts.external {
            resources.push((pattern, None));
        }

        let (router, resources) = Router::new(&prefix, parts.settings, resources);

        let inner = Rc::new(UnsafeCell::new(Inner {
            prefix: prefix_len,
            default: parts.default,
            encoding: parts.encoding,
            handlers: parts.handlers,
            resources,
        }));
        let filters = if parts.filters.is_empty() {
            None
        } else {
            Some(parts.filters)
        };

        HttpApplication {
            state: Rc::new(parts.state),
            router: router.clone(),
            middlewares: Rc::new(RefCell::new(parts.middlewares)),
            prefix,
            prefix_len,
            inner,
            filters,
        }
    }

    /// Convenience method for creating `Box<HttpHandler>` instances.
    ///
    /// This method is useful if you need to register multiple
    /// application instances with different state.
    ///
    /// ```rust
    /// # use std::thread;
    /// # extern crate actix_web;
    /// use actix_web::{server, App, HttpResponse};
    ///
    /// struct State1;
    ///
    /// struct State2;
    ///
    /// fn main() {
    /// # thread::spawn(|| {
    ///     server::new(|| {
    ///         vec![
    ///             App::with_state(State1)
    ///                 .prefix("/app1")
    ///                 .resource("/", |r| r.f(|r| HttpResponse::Ok()))
    ///                 .boxed(),
    ///             App::with_state(State2)
    ///                 .prefix("/app2")
    ///                 .resource("/", |r| r.f(|r| HttpResponse::Ok()))
    ///                 .boxed(),
    ///         ]
    ///     }).bind("127.0.0.1:8080")
    ///         .unwrap()
    ///         .run()
    /// # });
    /// }
    /// ```
    pub fn boxed(mut self) -> Box<HttpHandler<Task = Box<HttpHandlerTask>>> {
        Box::new(BoxedApplication { app: self.finish() })
    }
}

struct BoxedApplication<S> {
    app: HttpApplication<S>,
}

impl<S: 'static> HttpHandler for BoxedApplication<S> {
    type Task = Box<HttpHandlerTask>;

    fn handle(&mut self, req: HttpRequest) -> Result<Self::Task, HttpRequest> {
        self.app.handle(req).map(|t| {
            let task: Self::Task = Box::new(t);
            task
        })
    }
}

impl<S: 'static> IntoHttpHandler for App<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(mut self, settings: ServerSettings) -> HttpApplication<S> {
        {
            let parts = self.parts.as_mut().expect("Use after finish");
            parts.settings = settings;
        }
        self.finish()
    }
}

impl<'a, S: 'static> IntoHttpHandler for &'a mut App<S> {
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
impl<S: 'static> Iterator for App<S> {
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
    use super::*;
    use body::{Binary, Body};
    use http::StatusCode;
    use httprequest::HttpRequest;
    use httpresponse::HttpResponse;
    use pred;
    use test::{TestRequest, TestServer};

    #[test]
    fn test_default_resource() {
        let mut app = App::new()
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .finish();

        let req = TestRequest::with_uri("/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let mut app = App::new()
            .default_resource(|r| r.f(|_| HttpResponse::MethodNotAllowed()))
            .finish();
        let req = TestRequest::with_uri("/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_unhandled_prefix() {
        let mut app = App::new()
            .prefix("/test")
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        assert!(app.handle(HttpRequest::default()).is_err());
    }

    #[test]
    fn test_state() {
        let mut app = App::with_state(10)
            .resource("/", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        let req =
            HttpRequest::default().with_state(Rc::clone(&app.state), app.router.clone());
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_prefix() {
        let mut app = App::new()
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
        let mut app = App::new().handler("/test", |_| HttpResponse::Ok()).finish();

        let req = TestRequest::with_uri("/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/app").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/testapp").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handler2() {
        let mut app = App::new().handler("test", |_| HttpResponse::Ok()).finish();

        let req = TestRequest::with_uri("/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/app").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/testapp").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handler_with_prefix() {
        let mut app = App::new()
            .prefix("prefix")
            .handler("/test", |_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/prefix/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/prefix/test/").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/prefix/test/app").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/prefix/testapp").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/prefix/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_route() {
        let mut app = App::new()
            .route("/test", Method::GET, |_: HttpRequest| HttpResponse::Ok())
            .route("/test", Method::POST, |_: HttpRequest| {
                HttpResponse::Created()
            })
            .finish();

        let req = TestRequest::with_uri("/test").method(Method::GET).finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test").method(Method::POST).finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test").method(Method::HEAD).finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handler_prefix() {
        let mut app = App::new()
            .prefix("/app")
            .handler("/test", |_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/test").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/test").finish();
        let resp = app.run(req.clone());
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
        assert_eq!(req.prefix_len(), 9);

        let req = TestRequest::with_uri("/app/test/").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/test/app").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/testapp").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/blah").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_filter() {
        let mut srv = TestServer::with_factory(|| {
            App::new()
                .filter(pred::Get())
                .handler("/test", |_| HttpResponse::Ok())
        });

        let request = srv.get().uri(srv.url("/test")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let request = srv.post().uri(srv.url("/test")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_option_responder() {
        let mut app = App::new()
            .resource("/none", |r| r.f(|_| -> Option<&'static str> { None }))
            .resource("/some", |r| r.f(|_| Some("some")))
            .finish();

        let req = TestRequest::with_uri("/none").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/some").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
        assert_eq!(resp.as_msg().body(), &Body::Binary(Binary::Slice(b"some")));
    }
}
