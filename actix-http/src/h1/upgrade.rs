use std::marker::PhantomData;

use actix_codec::Framed;
use actix_server_config::ServerConfig;
use actix_service::{NewService, Service};
use futures::future::FutureResult;
use futures::{Async, Poll};

use crate::error::Error;
use crate::h1::Codec;
use crate::request::Request;

pub struct UpgradeHandler<T>(PhantomData<T>);

impl<T> NewService for UpgradeHandler<T> {
    type Config = ServerConfig;
    type Request = (Request, Framed<T, Codec>);
    type Response = ();
    type Error = Error;
    type Service = UpgradeHandler<T>;
    type InitError = Error;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &ServerConfig) -> Self::Future {
        unimplemented!()
    }
}

impl<T> Service for UpgradeHandler<T> {
    type Request = (Request, Framed<T, Codec>);
    type Response = ();
    type Error = Error;
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, _: Self::Request) -> Self::Future {
        unimplemented!()
    }
}
