use std::ops::Deref;
use std::rc::Rc;

use actix_http::error::{Error, ErrorInternalServerError};
use actix_http::Extensions;
use futures::future::{err, ok, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

use crate::extract::FromRequest;
use crate::service::ServiceFromRequest;

/// Application state factory
pub(crate) trait StateFactory {
    fn construct(&self) -> Box<StateFactoryResult>;
}

pub(crate) trait StateFactoryResult {
    fn poll_result(&mut self, extensions: &mut Extensions) -> Poll<(), ()>;
}

/// Application state
pub struct State<S>(Rc<S>);

impl<S> State<S> {
    pub fn new(state: S) -> State<S> {
        State(Rc::new(state))
    }

    pub fn get_ref(&self) -> &S {
        self.0.as_ref()
    }
}

impl<S> Deref for State<S> {
    type Target = S;

    fn deref(&self) -> &S {
        self.0.as_ref()
    }
}

impl<S> Clone for State<S> {
    fn clone(&self) -> State<S> {
        State(self.0.clone())
    }
}

impl<S: 'static, P> FromRequest<P> for State<S> {
    type Error = Error;
    type Future = FutureResult<Self, Error>;
    type Config = ();

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        if let Some(st) = req.app_extensions().get::<State<S>>() {
            ok(st.clone())
        } else {
            err(ErrorInternalServerError(
                "State is not configured, use App::state()",
            ))
        }
    }
}

impl<S: 'static> StateFactory for State<S> {
    fn construct(&self) -> Box<StateFactoryResult> {
        Box::new(StateFut { st: self.clone() })
    }
}

struct StateFut<S> {
    st: State<S>,
}

impl<S: 'static> StateFactoryResult for StateFut<S> {
    fn poll_result(&mut self, extensions: &mut Extensions) -> Poll<(), ()> {
        extensions.insert(self.st.clone());
        Ok(Async::Ready(()))
    }
}

impl<F, Out> StateFactory for F
where
    F: Fn() -> Out + 'static,
    Out: IntoFuture + 'static,
    Out::Error: std::fmt::Debug,
{
    fn construct(&self) -> Box<StateFactoryResult> {
        Box::new(StateFactoryFut {
            fut: (*self)().into_future(),
        })
    }
}

struct StateFactoryFut<S, F>
where
    F: Future<Item = S>,
    F::Error: std::fmt::Debug,
{
    fut: F,
}

impl<S: 'static, F> StateFactoryResult for StateFactoryFut<S, F>
where
    F: Future<Item = S>,
    F::Error: std::fmt::Debug,
{
    fn poll_result(&mut self, extensions: &mut Extensions) -> Poll<(), ()> {
        match self.fut.poll() {
            Ok(Async::Ready(s)) => {
                extensions.insert(State::new(s));
                Ok(Async::Ready(()))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                log::error!("Can not construct application state: {:?}", e);
                Err(())
            }
        }
    }
}
