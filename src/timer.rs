use std::time::{Duration, Instant};

use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll};
use tokio_current_thread::spawn;
use tokio_timer::sleep;

use super::cell::Cell;
use super::service::{NewService, Service};
use super::Never;

#[derive(Clone, Debug)]
pub struct LowResTimer(Cell<Inner>);

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

impl LowResTimer {
    pub fn with(resolution: Duration) -> LowResTimer {
        LowResTimer(Cell::new(Inner::new(resolution)))
    }

    pub fn timer(&self) -> LowResTimerService {
        LowResTimerService(self.0.clone())
    }
}

impl Default for LowResTimer {
    fn default() -> Self {
        LowResTimer(Cell::new(Inner::new(Duration::from_secs(1))))
    }
}

impl NewService for LowResTimer {
    type Request = ();
    type Response = Instant;
    type Error = Never;
    type InitError = Never;
    type Service = LowResTimerService;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(self.timer())
    }
}

#[derive(Clone, Debug)]
pub struct LowResTimerService(Cell<Inner>);

impl LowResTimerService {
    pub fn with(resolution: Duration) -> LowResTimerService {
        LowResTimerService(Cell::new(Inner::new(resolution)))
    }

    /// Get current time. This function has to be called from
    /// future's poll method, otherwise it panics.
    pub fn now(&self) -> Instant {
        let cur = self.0.borrow().current.clone();
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

impl Service for LowResTimerService {
    type Request = ();
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
