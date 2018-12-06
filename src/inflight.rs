use futures::{try_ready, Async, Future, Poll};

use super::counter::{Counter, CounterGuard};
use super::service::{IntoNewService, IntoService, NewService, Service};

/// InFlight - new service for service that can limit number of in-flight
/// async requests.
///
/// Default number of in-flight requests is 15
pub struct InFlight<T> {
    factory: T,
    max_inflight: usize,
}

impl<T> InFlight<T> {
    pub fn new<F, Request>(factory: F) -> Self
    where
        T: NewService<Request>,
        F: IntoNewService<T, Request>,
    {
        Self {
            factory: factory.into_new_service(),
            max_inflight: 15,
        }
    }

    /// Set max number of in-flight requests.
    ///
    /// By default max in-flight requests is 15.
    pub fn max_inflight(mut self, max: usize) -> Self {
        self.max_inflight = max;
        self
    }
}

impl<T, Request> NewService<Request> for InFlight<T>
where
    T: NewService<Request>,
{
    type Response = T::Response;
    type Error = T::Error;
    type InitError = T::InitError;
    type Service = InFlightService<T::Service>;
    type Future = InFlightResponseFuture<T, Request>;

    fn new_service(&self) -> Self::Future {
        InFlightResponseFuture {
            fut: self.factory.new_service(),
            max_inflight: self.max_inflight,
        }
    }
}

pub struct InFlightResponseFuture<T: NewService<Request>, Request> {
    fut: T::Future,
    max_inflight: usize,
}

impl<T: NewService<Request>, Request> Future for InFlightResponseFuture<T, Request> {
    type Item = InFlightService<T::Service>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(InFlightService::with_max_inflight(
            self.max_inflight,
            try_ready!(self.fut.poll()),
        )))
    }
}

pub struct InFlightService<T> {
    service: T,
    count: Counter,
}

impl<T> InFlightService<T> {
    pub fn new<F, Request>(service: F) -> Self
    where
        T: Service<Request>,
        F: IntoService<T, Request>,
    {
        Self {
            service: service.into_service(),
            count: Counter::new(15),
        }
    }

    pub fn with_max_inflight<F, Request>(max: usize, service: F) -> Self
    where
        T: Service<Request>,
        F: IntoService<T, Request>,
    {
        Self {
            service: service.into_service(),
            count: Counter::new(max),
        }
    }
}

impl<T, Request> Service<Request> for InFlightService<T>
where
    T: Service<Request>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = InFlightServiceResponse<T, Request>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        let res = self.service.poll_ready();
        if res.is_ok() && !self.count.available() {
            return Ok(Async::NotReady);
        }
        res
    }

    fn call(&mut self, req: Request) -> Self::Future {
        InFlightServiceResponse {
            fut: self.service.call(req),
            _guard: self.count.get(),
        }
    }
}

#[doc(hidden)]
pub struct InFlightServiceResponse<T: Service<Request>, Request> {
    fut: T::Future,
    _guard: CounterGuard,
}

impl<T: Service<Request>, Request> Future for InFlightServiceResponse<T, Request> {
    type Item = T::Response;
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll()
    }
}
