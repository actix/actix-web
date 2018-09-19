use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::{Future, Poll, Async};
use futures::future::{ok, FutureResult};
use tokio_current_thread::spawn;
use tokio_timer::sleep;

use super::service::{Service, NewService};

#[derive(Clone, Debug)]
pub struct LowResTimer(Rc<RefCell<Inner>>);

#[derive(Debug)]
struct Inner {
    interval: Duration,
    current: Option<Instant>,
}

impl Inner {
    fn new(interval: Duration) -> Self {
        Inner {
            interval,
            current: None
        }
    }
}

impl LowResTimer {
    pub fn with_interval(interval: Duration) -> LowResTimer {
        LowResTimer(Rc::new(RefCell::new(Inner::new(interval))))
    }
}

impl Default for LowResTimer {
    fn default() -> Self {
        LowResTimer(Rc::new(RefCell::new(Inner::new(Duration::from_secs(1)))))
    }
}

impl NewService for LowResTimer {
    type Request = ();
    type Response = Instant;
    type Error = ();
    type InitError = ();
    type Service = LowResTimerService;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(LowResTimerService(self.0.clone()))
    }
}


#[derive(Clone, Debug)]
pub struct LowResTimerService(Rc<RefCell<Inner>>);

impl LowResTimerService {
    pub fn with_interval(interval: Duration) -> LowResTimerService {
        LowResTimerService(Rc::new(RefCell::new(Inner::new(interval))))
    }
}

impl Service for LowResTimerService {
    type Request = ();
    type Response = Instant;
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, _: ()) -> Self::Future {
        let cur = self.0.borrow().current.clone();
        if let Some(cur) = cur {
            ok(cur)
        } else {
            let now = Instant::now();
            let inner = self.0.clone();
            let interval = {
                let mut b = inner.borrow_mut();
                b.current = Some(now);
                b.interval
            };

            spawn(
                sleep(interval)
                    .map_err(|_| panic!())
                    .and_then(move|_| {
                        inner.borrow_mut().current.take();
                        Ok(())
                    }));
            ok(now)
        }
    }
}
