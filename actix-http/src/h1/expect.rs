use std::task::{Context, Poll};

use actix_service::{Service, ServiceFactory};
use futures_util::future::{ok, Ready};

use crate::error::Error;
use crate::request::Request;

pub struct ExpectHandler;

impl ServiceFactory for ExpectHandler {
    type Config = ();
    type Request = Request;
    type Response = Request;
    type Error = Error;
    type Service = ExpectHandler;
    type InitError = Error;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(ExpectHandler)
    }
}

impl Service for ExpectHandler {
    type Request = Request;
    type Response = Request;
    type Error = Error;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        ok(req)
    }
}
