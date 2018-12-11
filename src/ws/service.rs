use std::marker::PhantomData;

use actix_codec::Framed;
use actix_service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, IntoFuture, Poll};

use crate::h1::Codec;
use crate::request::Request;

use super::{verify_handshake, HandshakeError};

pub struct VerifyWebSockets<T> {
    _t: PhantomData<T>,
}

impl<T> Default for VerifyWebSockets<T> {
    fn default() -> Self {
        VerifyWebSockets { _t: PhantomData }
    }
}

impl<T> NewService<(Request, Framed<T, Codec>)> for VerifyWebSockets<T> {
    type Response = (Request, Framed<T, Codec>);
    type Error = (HandshakeError, Framed<T, Codec>);
    type InitError = ();
    type Service = VerifyWebSockets<T>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(VerifyWebSockets { _t: PhantomData })
    }
}

impl<T> Service<(Request, Framed<T, Codec>)> for VerifyWebSockets<T> {
    type Response = (Request, Framed<T, Codec>);
    type Error = (HandshakeError, Framed<T, Codec>);
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, framed): (Request, Framed<T, Codec>)) -> Self::Future {
        match verify_handshake(&req) {
            Err(e) => Err((e, framed)).into_future(),
            Ok(_) => Ok((req, framed)).into_future(),
        }
    }
}
