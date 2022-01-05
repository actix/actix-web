use actix_codec::Framed;
use actix_service::{Service, ServiceFactory};
use futures_core::future::LocalBoxFuture;

use crate::{h1::Codec, Error, Request};

pub struct UpgradeHandler;

impl<T> ServiceFactory<(Request, Framed<T, Codec>)> for UpgradeHandler {
    type Response = ();
    type Error = Error;
    type Config = ();
    type Service = UpgradeHandler;
    type InitError = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        unimplemented!()
    }
}

impl<T> Service<(Request, Framed<T, Codec>)> for UpgradeHandler {
    type Response = ();
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, _: (Request, Framed<T, Codec>)) -> Self::Future {
        unimplemented!()
    }
}
