use actix_codec::Framed;
use actix_http::{h1::Codec, Request};

use crate::state::State;

pub struct FramedRequest<Io, S = ()> {
    req: Request,
    framed: Framed<Io, Codec>,
    state: State<S>,
}

impl<Io, S> FramedRequest<Io, S> {
    pub fn new(req: Request, framed: Framed<Io, Codec>, state: State<S>) -> Self {
        Self { req, framed, state }
    }
}

impl<Io, S> FramedRequest<Io, S> {
    pub fn request(&self) -> &Request {
        &self.req
    }

    pub fn request_mut(&mut self) -> &mut Request {
        &mut self.req
    }

    pub fn into_parts(self) -> (Request, Framed<Io, Codec>, State<S>) {
        (self.req, self.framed, self.state)
    }
}
