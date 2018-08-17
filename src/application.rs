use std::rc::Rc;

use handler::{AsyncResult, FromRequest, Handler, Responder, WrapHandler};
use header::ContentEncoding;
use http::Method;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::Middleware;
use pipeline::{Pipeline, PipelineHandler};
use pred::Predicate;
use resource::Resource;
use router::{ResourceDef, Router};
use scope::Scope;
use server::{HttpHandler, HttpHandlerTask, IntoHttpHandler, Request};
use with::WithFactory;

/// Application
pub struct HttpApplication<S = ()> {
    state: Rc<S>,
    prefix: String,
    prefix_len: usize,
    inner: Rc<Inner<S>>,
    filters: Option<Vec<Box<Predicate<S>>>>,
    middlewares: Rc<Vec<Box<Middleware<S>>>>,
}

#[doc(hidden)]
pub struct Inner<S> {
    router: Router<S>,
    encoding: ContentEncoding,
}

impl<S: 'static> PipelineHandler<S> for Inner<S> {
    #[inline]
    fn encoding(&self) -> ContentEncoding {
        self.encoding
    }

    fn handle(&self, req: &HttpRequest<S>) -> AsyncResult<HttpResponse> {
        self.router.handle(req)
    }
}

impl<S: 'static> HttpApplication<S> {
    #[cfg(test)]
    pub(crate) fn run(&self, req: Request) -> AsyncResult<HttpResponse> {
        let info = self
            .inner
            .router
            .recognize(&req, &self.state, self.prefix_len);
        let req = HttpRequest::new(req, Rc::clone(&self.state), info);

        self.inner.handle(&req)
    }
}

impl<S: 'static> HttpHandler for HttpApplication<S> {
    type Task = Pipeline<S, Inner<S>>;

    fn handle(&self, msg: Request) -> Result<Pipeline<S, Inner<S>>, Request> {
        let m = {
            if self.prefix_len == 0 {
                true
            } else {
                let path = msg.path();
                path.starts_with(&self.prefix)
                    && (path.len() == self.prefix_len
                        || path.split_at(self.prefix_len).1.starts_with('/'))
            }
        };
        if m {
            if let Some(ref filters) = self.filters {
                for filter in filters {
                    if !filter.check(&msg, &self.state) {
                        return Err(msg);
                    }
                }
            }

            let info = self
                .inner
                .router
                .recognize(&msg, &self.state, self.prefix_len);

            let inner = Rc::clone(&self.inner);
            let req = HttpRequest::new(msg, Rc::clone(&self.state), info);
            Ok(Pipeline::new(req, Rc::clone(&self.middlewares), inner))
        } else {
            Err(msg)
        }
    }
}

struct ApplicationParts<S> {
    state: S,
    prefix: String,
    router: Router<S>,
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
                prefix: "".to_owned(),
                router: Router::new(ResourceDef::prefix("")),
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
    /// `/other/...` would return `NOT FOUND`. It is also possible to
    /// handle `/app` path, to do this you can register resource for
    /// empty string `""`
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .prefix("/app")
    ///         .resource("", |r| r.f(|_| HttpResponse::Ok()))  // <- handle `/app` path
    ///         .resource("/", |r| r.f(|_| HttpResponse::Ok())) // <- handle `/app/` path
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
            parts.router.set_prefix(&prefix);
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
        F: WithFactory<T, S, R>,
        R: Responder + 'static,
        T: FromRequest<S> + 'static,
    {
        self.parts
            .as_mut()
            .expect("Use after finish")
            .router
            .register_route(path, method, f);

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
        let scope = f(Scope::new(path));
        self.parts
            .as_mut()
            .expect("Use after finish")
            .router
            .register_scope(scope);
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
        F: FnOnce(&mut Resource<S>) -> R + 'static,
    {
        {
            let parts = self.parts.as_mut().expect("Use after finish");

            // create resource
            let mut resource = Resource::new(ResourceDef::new(path));

            // configure
            f(&mut resource);

            parts.router.register_resource(resource);
        }
        self
    }

    /// Configure resource for a specific path.
    #[doc(hidden)]
    pub fn register_resource(&mut self, resource: Resource<S>) {
        self.parts
            .as_mut()
            .expect("Use after finish")
            .router
            .register_resource(resource);
    }

    /// Default resource to be used if no matching route could be found.
    pub fn default_resource<F, R>(mut self, f: F) -> App<S>
    where
        F: FnOnce(&mut Resource<S>) -> R + 'static,
    {
        // create and configure default resource
        let mut resource = Resource::new(ResourceDef::new(""));
        f(&mut resource);

        self.parts
            .as_mut()
            .expect("Use after finish")
            .router
            .register_default_resource(resource.into());

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
    /// fn index(req: &HttpRequest) -> Result<HttpResponse> {
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
        self.parts
            .as_mut()
            .expect("Use after finish")
            .router
            .register_external(name.as_ref(), ResourceDef::external(url.as_ref()));
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
    ///     let app = App::new().handler("/app", |req: &HttpRequest| match *req.method() {
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
            self.parts
                .as_mut()
                .expect("Use after finish")
                .router
                .register_handler(&path, Box::new(WrapHandler::new(handler)), None);
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
    ///         .handler("/static", fs::StaticFiles::new(".").unwrap());
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
        let mut parts = self.parts.take().expect("Use after finish");
        let prefix = parts.prefix.trim().trim_right_matches('/');
        parts.router.finish();

        let inner = Rc::new(Inner {
            router: parts.router,
            encoding: parts.encoding,
        });
        let filters = if parts.filters.is_empty() {
            None
        } else {
            Some(parts.filters)
        };

        HttpApplication {
            inner,
            filters,
            state: Rc::new(parts.state),
            middlewares: Rc::new(parts.middlewares),
            prefix: prefix.to_owned(),
            prefix_len: prefix.len(),
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

    fn handle(&self, req: Request) -> Result<Self::Task, Request> {
        self.app.handle(req).map(|t| {
            let task: Self::Task = Box::new(t);
            task
        })
    }
}

impl<S: 'static> IntoHttpHandler for App<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(mut self) -> HttpApplication<S> {
        self.finish()
    }
}

impl<'a, S: 'static> IntoHttpHandler for &'a mut App<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(self) -> HttpApplication<S> {
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
        let app = App::new()
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .finish();

        let req = TestRequest::with_uri("/test").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let app = App::new()
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .default_resource(|r| r.f(|_| HttpResponse::MethodNotAllowed()))
            .finish();
        let req = TestRequest::with_uri("/blah").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_unhandled_prefix() {
        let app = App::new()
            .prefix("/test")
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        let ctx = TestRequest::default().request();
        assert!(app.handle(ctx).is_err());
    }

    #[test]
    fn test_state() {
        let app = App::with_state(10)
            .resource("/", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        let req = TestRequest::with_state(10).request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_prefix() {
        let app = App::new()
            .prefix("/test")
            .resource("/blah", |r| r.f(|_| HttpResponse::Ok()))
            .finish();
        let req = TestRequest::with_uri("/test").request();
        let resp = app.handle(req);
        assert!(resp.is_ok());

        let req = TestRequest::with_uri("/test/").request();
        let resp = app.handle(req);
        assert!(resp.is_ok());

        let req = TestRequest::with_uri("/test/blah").request();
        let resp = app.handle(req);
        assert!(resp.is_ok());

        let req = TestRequest::with_uri("/testing").request();
        let resp = app.handle(req);
        assert!(resp.is_err());
    }

    #[test]
    fn test_handler() {
        let app = App::new()
            .handler("/test", |_: &_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/test").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/testapp").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/blah").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handler2() {
        let app = App::new()
            .handler("test", |_: &_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/test").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/testapp").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/blah").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handler_with_prefix() {
        let app = App::new()
            .prefix("prefix")
            .handler("/test", |_: &_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/prefix/test").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/prefix/test/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/prefix/test/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/prefix/testapp").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/prefix/blah").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_route() {
        let app = App::new()
            .route("/test", Method::GET, |_: HttpRequest| HttpResponse::Ok())
            .route("/test", Method::POST, |_: HttpRequest| {
                HttpResponse::Created()
            })
            .finish();

        let req = TestRequest::with_uri("/test").method(Method::GET).request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test")
            .method(Method::HEAD)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handler_prefix() {
        let app = App::new()
            .prefix("/app")
            .handler("/test", |_: &_| HttpResponse::Ok())
            .finish();

        let req = TestRequest::with_uri("/test").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/test").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/test/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/test/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/testapp").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/blah").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_option_responder() {
        let app = App::new()
            .resource("/none", |r| r.f(|_| -> Option<&'static str> { None }))
            .resource("/some", |r| r.f(|_| Some("some")))
            .finish();

        let req = TestRequest::with_uri("/none").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/some").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
        assert_eq!(resp.as_msg().body(), &Body::Binary(Binary::Slice(b"some")));
    }

    #[test]
    fn test_filter() {
        let mut srv = TestServer::with_factory(|| {
            App::new()
                .filter(pred::Get())
                .handler("/test", |_: &_| HttpResponse::Ok())
        });

        let request = srv.get().uri(srv.url("/test")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let request = srv.post().uri(srv.url("/test")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_prefix_root() {
        let mut srv = TestServer::with_factory(|| {
            App::new()
                .prefix("/test")
                .resource("/", |r| r.f(|_| HttpResponse::Ok()))
                .resource("", |r| r.f(|_| HttpResponse::Created()))
        });

        let request = srv.get().uri(srv.url("/test/")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let request = srv.get().uri(srv.url("/test")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

}
