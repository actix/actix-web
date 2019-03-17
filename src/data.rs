use std::ops::Deref;
use std::sync::Arc;

use actix_http::error::{Error, ErrorInternalServerError};
use actix_http::Extensions;
use futures::{Async, Future, IntoFuture, Poll};

use crate::extract::FromRequest;
use crate::service::ServiceFromRequest;

/// Application data factory
pub(crate) trait DataFactory {
    fn construct(&self) -> Box<DataFactoryResult>;
}

pub(crate) trait DataFactoryResult {
    fn poll_result(&mut self, extensions: &mut Extensions) -> Poll<(), ()>;
}

/// Application state
pub struct Data<T>(Arc<T>);

impl<T> Data<T> {
    pub(crate) fn new(state: T) -> Data<T> {
        Data(Arc::new(state))
    }

    /// Get referecnce to inner state type.
    pub fn get_ref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T> Deref for Data<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T> Clone for Data<T> {
    fn clone(&self) -> Data<T> {
        Data(self.0.clone())
    }
}

impl<T: 'static, P> FromRequest<P> for Data<T> {
    type Error = Error;
    type Future = Result<Self, Error>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        if let Some(st) = req.config().extensions().get::<Data<T>>() {
            Ok(st.clone())
        } else {
            Err(ErrorInternalServerError(
                "State is not configured, to configure use App::state()",
            ))
        }
    }
}

impl<T: 'static> DataFactory for Data<T> {
    fn construct(&self) -> Box<DataFactoryResult> {
        Box::new(DataFut { st: self.clone() })
    }
}

struct DataFut<T> {
    st: Data<T>,
}

impl<T: 'static> DataFactoryResult for DataFut<T> {
    fn poll_result(&mut self, extensions: &mut Extensions) -> Poll<(), ()> {
        extensions.insert(self.st.clone());
        Ok(Async::Ready(()))
    }
}

impl<F, Out> DataFactory for F
where
    F: Fn() -> Out + 'static,
    Out: IntoFuture + 'static,
    Out::Error: std::fmt::Debug,
{
    fn construct(&self) -> Box<DataFactoryResult> {
        Box::new(DataFactoryFut {
            fut: (*self)().into_future(),
        })
    }
}

struct DataFactoryFut<T, F>
where
    F: Future<Item = T>,
    F::Error: std::fmt::Debug,
{
    fut: F,
}

impl<T: 'static, F> DataFactoryResult for DataFactoryFut<T, F>
where
    F: Future<Item = T>,
    F::Error: std::fmt::Debug,
{
    fn poll_result(&mut self, extensions: &mut Extensions) -> Poll<(), ()> {
        match self.fut.poll() {
            Ok(Async::Ready(s)) => {
                extensions.insert(Data::new(s));
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
