use std::ops::Deref;
use std::sync::Arc;

use actix_http::error::{Error, ErrorInternalServerError};
use actix_http::Extensions;
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
pub struct State<T>(Arc<T>);

impl<T> State<T> {
    pub(crate) fn new(state: T) -> State<T> {
        State(Arc::new(state))
    }

    /// Get referecnce to inner state type.
    pub fn get_ref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T> Deref for State<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T> Clone for State<T> {
    fn clone(&self) -> State<T> {
        State(self.0.clone())
    }
}

impl<T: 'static, P> FromRequest<P> for State<T> {
    type Error = Error;
    type Future = Result<Self, Error>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        if let Some(st) = req.config().extensions().get::<State<T>>() {
            Ok(st.clone())
        } else {
            Err(ErrorInternalServerError(
                "State is not configured, to configure use App::state()",
            ))
        }
    }
}

impl<T: 'static> StateFactory for State<T> {
    fn construct(&self) -> Box<StateFactoryResult> {
        Box::new(StateFut { st: self.clone() })
    }
}

struct StateFut<T> {
    st: State<T>,
}

impl<T: 'static> StateFactoryResult for StateFut<T> {
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

struct StateFactoryFut<T, F>
where
    F: Future<Item = T>,
    F::Error: std::fmt::Debug,
{
    fut: F,
}

impl<T: 'static, F> StateFactoryResult for StateFactoryFut<T, F>
where
    F: Future<Item = T>,
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
