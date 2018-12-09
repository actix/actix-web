use std::marker::PhantomData;
use std::time::{Duration, Instant};

use actix_service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll};
use tokio_timer::Delay;

use super::time::{LowResTime, LowResTimeService};
use super::Never;

pub struct KeepAlive<R, E, F> {
    f: F,
    ka: Duration,
    time: LowResTime,
    _t: PhantomData<(R, E)>,
}

impl<R, E, F> KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    pub fn new(ka: Duration, time: LowResTime, f: F) -> Self {
        KeepAlive {
            f,
            ka,
            time,
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Clone for KeepAlive<R, E, F>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        KeepAlive {
            f: self.f.clone(),
            ka: self.ka,
            time: self.time.clone(),
            _t: PhantomData,
        }
    }
}

impl<R, E, F> NewService<R> for KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    type Response = R;
    type Error = E;
    type InitError = Never;
    type Service = KeepAliveService<R, E, F>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(KeepAliveService::new(
            self.ka,
            self.time.timer(),
            self.f.clone(),
        ))
    }
}

pub struct KeepAliveService<R, E, F> {
    f: F,
    ka: Duration,
    time: LowResTimeService,
    delay: Delay,
    expire: Instant,
    _t: PhantomData<(R, E)>,
}

impl<R, E, F> KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    pub fn new(ka: Duration, time: LowResTimeService, f: F) -> Self {
        let expire = time.now() + ka;
        KeepAliveService {
            f,
            ka,
            time,
            expire,
            delay: Delay::new(expire),
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Service<R> for KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    type Response = R;
    type Error = E;
    type Future = FutureResult<R, E>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        match self.delay.poll() {
            Ok(Async::Ready(_)) => {
                let now = self.time.now();
                if self.expire <= now {
                    Err((self.f)())
                } else {
                    self.delay.reset(self.expire);
                    let _ = self.delay.poll();
                    Ok(Async::Ready(()))
                }
            }
            Ok(Async::NotReady) => Ok(Async::Ready(())),
            Err(_) => panic!(),
        }
    }

    fn call(&mut self, req: R) -> Self::Future {
        self.expire = self.time.now() + self.ka;
        ok(req)
    }
}
