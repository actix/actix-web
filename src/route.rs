use futures::Future;

use error::Error;
use pred::Predicate;
use handler::{Reply, Handler, Responder, RouteHandler, AsyncHandler, WrapHandler};
use httpcodes::HTTPNotFound;
use httprequest::HttpRequest;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route<S> {
    preds: Vec<Box<Predicate<S>>>,
    handler: Box<RouteHandler<S>>,
}

impl<S: 'static> Default for Route<S> {

    fn default() -> Route<S> {
        Route {
            preds: Vec::new(),
            handler: Box::new(WrapHandler::new(|_| HTTPNotFound)),
        }
    }
}

impl<S: 'static> Route<S> {

    pub(crate) fn check(&self, req: &mut HttpRequest<S>) -> bool {
        for pred in &self.preds {
            if !pred.check(req) {
                return false
            }
        }
        true
    }

    pub(crate) fn handle(&self, req: HttpRequest<S>) -> Reply {
        self.handler.handle(req)
    }

    /// Add match predicate to route.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # use actix_web::httpcodes::*;
    /// # fn main() {
    /// Application::new()
    ///    .resource("/path", |r|
    ///       r.route()
    ///          .p(pred::Get())
    ///          .p(pred::Header("content-type", "text/plain"))
    ///          .f(|req| HTTPOk)
    ///       )
    /// #      .finish();
    /// # }
    /// ```
    pub fn p<T: Predicate<S> + 'static>(&mut self, p: T) -> &mut Self {
        self.preds.push(Box::new(p));
        self
    }

    /// Set handler object. Usually call to this method is last call
    /// during route configuration, because it does not return reference to self.
    pub fn h<H: Handler<S>>(&mut self, handler: H) {
        self.handler = Box::new(WrapHandler::new(handler));
    }

    /// Set handler function. Usually call to this method is last call
    /// during route configuration, because it does not return reference to self.
    pub fn f<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Responder + 'static,
    {
        self.handler = Box::new(WrapHandler::new(handler));
    }

    /// Set async handler function.
    pub fn a<H, R, F, E>(&mut self, handler: H)
        where H: Fn(HttpRequest<S>) -> F + 'static,
              F: Future<Item=R, Error=E> + 'static,
              R: Responder + 'static,
              E: Into<Error> + 'static
    {
        self.handler = Box::new(AsyncHandler::new(handler));
    }
}
