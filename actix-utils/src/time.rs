use std::time::{Duration, Instant};

use actix_rt::spawn;
use actix_service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll};
use tokio_timer::sleep;

use super::cell::Cell;
use super::Never;

#[derive(Clone, Debug)]
pub struct LowResTime(Cell<Inner>);

#[derive(Debug)]
struct Inner {
    resolution: Duration,
    current: Option<Instant>,
}

impl Inner {
    fn new(resolution: Duration) -> Self {
        Inner {
            resolution,
            current: None,
        }
    }
}

impl LowResTime {
    pub fn with(resolution: Duration) -> LowResTime {
        LowResTime(Cell::new(Inner::new(resolution)))
    }

    pub fn timer(&self) -> LowResTimeService {
        LowResTimeService(self.0.clone())
    }
}

impl Default for LowResTime {
    fn default() -> Self {
        LowResTime(Cell::new(Inner::new(Duration::from_secs(1))))
    }
}

impl NewService<()> for LowResTime {
    type Response = Instant;
    type Error = Never;
    type InitError = Never;
    type Service = LowResTimeService;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(self.timer())
    }
}

#[derive(Clone, Debug)]
pub struct LowResTimeService(Cell<Inner>);

impl LowResTimeService {
    pub fn with(resolution: Duration) -> LowResTimeService {
        LowResTimeService(Cell::new(Inner::new(resolution)))
    }

    /// Get current time. This function has to be called from
    /// future's poll method, otherwise it panics.
    pub fn now(&self) -> Instant {
        let cur = self.0.borrow().current;
        if let Some(cur) = cur {
            cur
        } else {
            let now = Instant::now();
            let inner = self.0.clone();
            let interval = {
                let mut b = inner.borrow_mut();
                b.current = Some(now);
                b.resolution
            };

            spawn(sleep(interval).map_err(|_| panic!()).and_then(move |_| {
                inner.borrow_mut().current.take();
                Ok(())
            }));
            now
        }
    }
}

impl Service<()> for LowResTimeService {
    type Response = Instant;
    type Error = Never;
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, _: ()) -> Self::Future {
        ok(self.now())
    }
}
