//! Service that applies a timeout to requests.
//!
//! If the response does not complete within the specified timeout, the response
//! will be aborted.
use std::fmt;
use std::time::Duration;

use futures::try_ready;
use futures::{Async, Future, Poll};
use tokio_timer::{clock, Delay};

use crate::service::{NewService, Service};

/// Applies a timeout to requests.
#[derive(Debug)]
pub struct Timeout<T> {
    inner: T,
    timeout: Duration,
}

/// Timeout error
pub enum TimeoutError<E> {
    /// Service error
    Service(E),
    /// Service call timeout
    Timeout,
}

impl<E: fmt::Debug> fmt::Debug for TimeoutError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TimeoutError::Service(e) => write!(f, "TimeoutError::Service({:?})", e),
            TimeoutError::Timeout => write!(f, "TimeoutError::Timeout"),
        }
    }
}

impl<T> Timeout<T> {
    pub fn new<Request>(timeout: Duration, inner: T) -> Self
    where
        T: NewService<Request> + Clone,
    {
        Timeout { inner, timeout }
    }
}

impl<T, Request> NewService<Request> for Timeout<T>
where
    T: NewService<Request> + Clone,
{
    type Response = T::Response;
    type Error = TimeoutError<T::Error>;
    type InitError = T::InitError;
    type Service = TimeoutService<T::Service>;
    type Future = TimeoutFut<T, Request>;

    fn new_service(&self) -> Self::Future {
        TimeoutFut {
            fut: self.inner.new_service(),
            timeout: self.timeout,
        }
    }
}

/// `Timeout` response future
#[derive(Debug)]
pub struct TimeoutFut<T: NewService<Request>, Request> {
    fut: T::Future,
    timeout: Duration,
}

impl<T, Request> Future for TimeoutFut<T, Request>
where
    T: NewService<Request>,
{
    type Item = TimeoutService<T::Service>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let service = try_ready!(self.fut.poll());
        Ok(Async::Ready(TimeoutService::new(self.timeout, service)))
    }
}

/// Applies a timeout to requests.
#[derive(Debug)]
pub struct TimeoutService<T> {
    inner: T,
    timeout: Duration,
}

impl<T> TimeoutService<T> {
    pub fn new<Request>(timeout: Duration, inner: T) -> Self
    where
        T: Service<Request>,
    {
        TimeoutService { inner, timeout }
    }
}

impl<T: Clone> Clone for TimeoutService<T> {
    fn clone(&self) -> Self {
        TimeoutService {
            inner: self.inner.clone(),
            timeout: self.timeout,
        }
    }
}

impl<T, Request> Service<Request> for TimeoutService<T>
where
    T: Service<Request>,
{
    type Response = T::Response;
    type Error = TimeoutError<T::Error>;
    type Future = TimeoutServiceResponse<T, Request>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready().map_err(TimeoutError::Service)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        TimeoutServiceResponse {
            fut: self.inner.call(request),
            sleep: Delay::new(clock::now() + self.timeout),
        }
    }
}

/// `TimeoutService` response future
#[derive(Debug)]
pub struct TimeoutServiceResponse<T: Service<Request>, Request> {
    fut: T::Future,
    sleep: Delay,
}

impl<T, Request> Future for TimeoutServiceResponse<T, Request>
where
    T: Service<Request>,
{
    type Item = T::Response;
    type Error = TimeoutError<T::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // First, try polling the future
        match self.fut.poll() {
            Ok(Async::Ready(v)) => return Ok(Async::Ready(v)),
            Ok(Async::NotReady) => {}
            Err(e) => return Err(TimeoutError::Service(e)),
        }

        // Now check the sleep
        match self.sleep.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(_)) => Err(TimeoutError::Timeout),
            Err(_) => Err(TimeoutError::Timeout),
        }
    }
}
