use actix_service::{Service, Transform};
use futures::future::{ok, Either, FutureResult, Map};
use futures::{Future, Poll};

/// `Middleware` for conditianally enables another middleware
/// The controled middleware must not change the `Service` interfaces.
/// This means you cannot control such middlewares like `Logger` or `Compress`.
///
/// ## Usage
///
/// ```
/// use actix_web::middleware::{Condition, NormalizePath};
/// use actix_web::App;
///
/// fn main() {
///     std::env::set_var("RUST_LOG", "actix_web=info");
///     env_logger::init();
///
///     let app = App::new()
///         .wrap(Condition::new(true, NormalizePath));
/// }
/// ```
///
pub struct Condition<T> {
    trans: T,
    enable: bool,
}

impl<T> Condition<T> {
    pub fn new(enable: bool, trans: T) -> Self {
        Self { trans, enable }
    }
}

impl<S, T> Transform<S> for Condition<T>
where
    S: Service,
    T: Transform<S, Request = S::Request, Response = S::Response, Error = S::Error>,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type InitError = T::InitError;
    type Transform = ConditionMiddleware<T::Transform, S>;
    type Future = Either<
        Map<T::Future, fn(T::Transform) -> Self::Transform>,
        FutureResult<Self::Transform, Self::InitError>,
    >;

    fn new_transform(&self, service: S) -> Self::Future {
        if self.enable {
            let f = self
                .trans
                .new_transform(service)
                .map(ConditionMiddleware::Enable as fn(T::Transform) -> Self::Transform);
            Either::A(f)
        } else {
            Either::B(ok(ConditionMiddleware::Disable(service)))
        }
    }
}

pub enum ConditionMiddleware<E, D> {
    Enable(E),
    Disable(D),
}

impl<E, D> Service for ConditionMiddleware<E, D>
where
    E: Service,
    D: Service<Request = E::Request, Response = E::Response, Error = E::Error>,
{
    type Request = E::Request;
    type Response = E::Response;
    type Error = E::Error;
    type Future = Either<E::Future, D::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        use ConditionMiddleware::*;
        match self {
            Enable(service) => service.poll_ready(),
            Disable(service) => service.poll_ready(),
        }
    }

    fn call(&mut self, req: E::Request) -> Self::Future {
        use ConditionMiddleware::*;
        match self {
            Enable(service) => Either::A(service.call(req)),
            Disable(service) => Either::B(service.call(req)),
        }
    }
}
