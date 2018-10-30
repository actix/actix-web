//! Service that applies a timeout to requests.
//!
//! If the response does not complete within the specified timeout, the response
//! will be aborted.
use std::fmt;
use std::time::Duration;

use futures::{Async, Future, Poll};
use tokio_timer::{clock, Delay};

use service::{NewService, Service};

/// Applies a timeout to requests.
#[derive(Debug)]
pub struct Timeout<T: NewService + Clone> {
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

impl<T> Timeout<T>
where
    T: NewService + Clone,
{
    pub fn new(timeout: Duration, inner: T) -> Self {
        Timeout { inner, timeout }
    }
}

impl<T> NewService for Timeout<T>
where
    T: NewService + Clone,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = TimeoutError<T::Error>;
    type InitError = T::InitError;
    type Service = TimeoutService<T::Service>;
    type Future = TimeoutFut<T>;

    fn new_service(&self) -> Self::Future {
        TimeoutFut {
            fut: self.inner.new_service(),
            timeout: self.timeout.clone(),
        }
    }
}

/// `Timeout` response future
#[derive(Debug)]
pub struct TimeoutFut<T: NewService> {
    fut: T::Future,
    timeout: Duration,
}

impl<T> Future for TimeoutFut<T>
where
    T: NewService,
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
    pub fn new(timeout: Duration, inner: T) -> Self {
        TimeoutService { inner, timeout }
    }
}

impl<T> Clone for TimeoutService<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        TimeoutService {
            inner: self.inner.clone(),
            timeout: self.timeout,
        }
    }
}

impl<T> Service for TimeoutService<T>
where
    T: Service,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = TimeoutError<T::Error>;
    type Future = TimeoutServiceResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner
            .poll_ready()
            .map_err(|e| TimeoutError::Service(e))
    }

    fn call(&mut self, request: Self::Request) -> Self::Future {
        TimeoutServiceResponse {
            fut: self.inner.call(request),
            sleep: Delay::new(clock::now() + self.timeout),
        }
    }
}

/// `TimeoutService` response future
#[derive(Debug)]
pub struct TimeoutServiceResponse<T: Service> {
    fut: T::Future,
    sleep: Delay,
}

impl<T> Future for TimeoutServiceResponse<T>
where
    T: Service,
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
