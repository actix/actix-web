use actix_server_config::ServerConfig;
use actix_service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Poll};

use crate::error::Error;
use crate::request::Request;

pub struct ExpectHandler;

impl NewService for ExpectHandler {
    type Config = ServerConfig;
    type Request = Request;
    type Response = Request;
    type Error = Error;
    type Service = ExpectHandler;
    type InitError = Error;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &ServerConfig) -> Self::Future {
        ok(ExpectHandler)
    }
}

impl Service for ExpectHandler {
    type Request = Request;
    type Response = Request;
    type Error = Error;
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        ok(req)
    }
}
