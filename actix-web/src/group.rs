use std::rc::Rc;

use actix_service::{apply, boxed, ServiceFactoryExt as _, Transform};

use crate::{
    body::MessageBody,
    config::AppService,
    dev::{ServiceRequest, ServiceResponse},
    service::{
        AppServiceFactory, BoxedHttpService, BoxedHttpServiceFactory, HttpServiceFactory,
        ServiceFactoryWrapper,
    },
    Error, Resource, Route,
};

type WrapFactory = Box<dyn Fn(BoxedHttpServiceFactory) -> BoxedHttpServiceFactory>;

/// A collection of services with common middleware and no path prefix.
///
/// Children register directly in the parent application or scope, in declaration order.
/// A group does not introduce a routing boundary or a default service. A child scope
/// retains its own prefix matching and default service behavior.
///
/// Middleware factories are shared across children, but each child gets its own
/// middleware service instance. Store shared state in an [`Rc`] or [`web::Data`](crate::web::Data).
/// The last middleware registered runs first on requests. Response bodies are boxed
/// between group middleware layers.
///
/// # Examples
/// ```
/// use actix_web::{middleware::DefaultHeaders, web, App, HttpResponse};
///
/// let app = App::new()
///     .route("/login", web::get().to(HttpResponse::Ok))
///     .service(
///         web::group()
///             .wrap(DefaultHeaders::new().add(("x-group", "private")))
///             .route("/dashboard", web::get().to(HttpResponse::Ok))
///             .service(web::scope("/users").route("/{id}", web::get().to(HttpResponse::Ok))),
///     );
/// ```
pub struct Group {
    services: Vec<Box<dyn AppServiceFactory>>,
    wrap: WrapFactory,
}

impl Group {
    /// Creates an empty group.
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
            wrap: Box::new(|factory| factory),
        }
    }

    /// Registers a resource, scope, group, or other HTTP service.
    pub fn service<F>(mut self, factory: F) -> Self
    where
        F: HttpServiceFactory + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Registers a route for a path.
    ///
    /// Like [`Scope::route`](crate::Scope::route), this registers a separate resource
    /// with the route's guards. Multiple calls can use the same path with different guards.
    pub fn route(self, path: &str, mut route: Route) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Applies middleware to each child, including children registered before this call.
    ///
    /// The middleware factory need not implement `Clone`. Its `new_transform` method
    /// runs once per child service each time the application is initialized.
    pub fn wrap<M, B>(self, mw: M) -> Self
    where
        M: Transform<
                BoxedHttpService,
                ServiceRequest,
                Response = ServiceResponse<B>,
                Error = Error,
                InitError = (),
            > + 'static,
        B: MessageBody + 'static,
    {
        let previous = self.wrap;
        let mw = Rc::new(mw);

        Self {
            services: self.services,
            wrap: Box::new(move |factory| {
                boxed::factory(
                    apply(Rc::clone(&mw), previous(factory)).map(|res| res.map_into_boxed_body()),
                )
            }),
        }
    }
}

impl Default for Group {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpServiceFactory for Group {
    fn register(self, config: &mut AppService) {
        let Self { services, wrap } = self;
        config.map_registered_services(
            move |config| {
                for mut service in services {
                    service.register(config);
                }
            },
            wrap,
        );
    }
}
