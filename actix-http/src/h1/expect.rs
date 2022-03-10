use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ready, Ready};

use crate::{Error, Request};

pub struct ExpectHandler;

impl ServiceFactory<Request> for ExpectHandler {
    type Response = Request;
    type Error = Error;
    type Config = ();
    type Service = ExpectHandler;
    type InitError = Error;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        ready(Ok(ExpectHandler))
    }
}

impl Service<Request> for ExpectHandler {
    type Response = Request;
    type Error = Error;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, req: Request) -> Self::Future {
        ready(Ok(req))
        // TODO: add some way to trigger error
        // Err(error::ErrorExpectationFailed("test"))
    }
}
