use std::marker::PhantomData;
use std::time::{Duration, Instant};

use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll};
use tokio_timer::Delay;

use super::service::{NewService, Service};
use super::timer::{LowResTimer, LowResTimerService};
use super::Never;

pub struct KeepAlive<R, E, F> {
    f: F,
    ka: Duration,
    timer: LowResTimer,
    _t: PhantomData<(R, E)>,
}

impl<R, E, F> KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    pub fn new(ka: Duration, timer: LowResTimer, f: F) -> Self {
        KeepAlive {
            f,
            ka,
            timer,
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Clone for KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    fn clone(&self) -> Self {
        KeepAlive {
            f: self.f.clone(),
            ka: self.ka,
            timer: self.timer.clone(),
            _t: PhantomData,
        }
    }
}

impl<R, E, F> NewService for KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    type Request = R;
    type Response = R;
    type Error = E;
    type InitError = Never;
    type Service = KeepAliveService<R, E, F>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(KeepAliveService::new(
            self.ka,
            self.timer.timer(),
            self.f.clone(),
        ))
    }
}

pub struct KeepAliveService<R, E, F> {
    f: F,
    ka: Duration,
    timer: LowResTimerService,
    delay: Delay,
    expire: Instant,
    _t: PhantomData<(R, E)>,
}

impl<R, E, F> KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    pub fn new(ka: Duration, mut timer: LowResTimerService, f: F) -> Self {
        let expire = timer.now() + ka;
        KeepAliveService {
            f,
            ka,
            timer,
            delay: Delay::new(expire),
            expire,
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Service for KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    type Request = R;
    type Response = R;
    type Error = E;
    type Future = FutureResult<R, E>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        match self.delay.poll() {
            Ok(Async::Ready(_)) => {
                let now = self.timer.now();
                if self.expire <= now {
                    Err((self.f)())
                } else {
                    self.delay = Delay::new(self.expire);
                    let _ = self.delay.poll();
                    Ok(Async::Ready(()))
                }
            }
            Ok(Async::NotReady) => Ok(Async::Ready(())),
            Err(_) => panic!(),
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.expire = self.timer.now() + self.ka;
        ok(req)
    }
}
