use std::task::{Context, Poll};

use actix_service::{Service, ServiceFactory};
use futures_util::future::{ready, Ready};

use crate::error::Error;
use crate::request::Request;

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

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        ready(Ok(req))
        // TODO: add some way to trigger error
        // Err(error::ErrorExpectationFailed("test"))
    }
}
