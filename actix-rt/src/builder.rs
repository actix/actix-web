use std::borrow::Cow;
use std::io;

use futures::future::{lazy, Future};
use futures::sync::mpsc::unbounded;
use futures::sync::oneshot::{channel, Receiver};

use tokio_current_thread::CurrentThread;
use tokio_reactor::Reactor;
use tokio_timer::clock::Clock;
use tokio_timer::timer::Timer;

use crate::arbiter::{Arbiter, SystemArbiter};
use crate::runtime::Runtime;
use crate::system::System;

/// Builder struct for a actix runtime.
///
/// Either use `Builder::build` to create a system and start actors.
/// Alternatively, use `Builder::run` to start the tokio runtime and
/// run a function in its context.
pub struct Builder {
    /// Name of the System. Defaults to "actix" if unset.
    name: Cow<'static, str>,

    /// The clock to use
    clock: Clock,

    /// Whether the Arbiter will stop the whole System on uncaught panic. Defaults to false.
    stop_on_panic: bool,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Builder {
            name: Cow::Borrowed("actix"),
            clock: Clock::new(),
            stop_on_panic: false,
        }
    }

    /// Sets the name of the System.
    pub fn name<T: Into<String>>(mut self, name: T) -> Self {
        self.name = Cow::Owned(name.into());
        self
    }

    /// Set the Clock instance that will be used by this System.
    ///
    /// Defaults to the system clock.
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Sets the option 'stop_on_panic' which controls whether the System is stopped when an
    /// uncaught panic is thrown from a worker thread.
    ///
    /// Defaults to false.
    pub fn stop_on_panic(mut self, stop_on_panic: bool) -> Self {
        self.stop_on_panic = stop_on_panic;
        self
    }

    /// Create new System.
    ///
    /// This method panics if it can not create tokio runtime
    pub fn build(self) -> SystemRunner {
        self.create_runtime(|| {})
    }

    /// This function will start tokio runtime and will finish once the
    /// `System::stop()` message get called.
    /// Function `f` get called within tokio runtime context.
    pub fn run<F>(self, f: F) -> i32
    where
        F: FnOnce() + 'static,
    {
        self.create_runtime(f).run()
    }

    fn create_runtime<F>(self, f: F) -> SystemRunner
    where
        F: FnOnce() + 'static,
    {
        let (stop_tx, stop) = channel();
        let (sys_sender, sys_receiver) = unbounded();

        let arbiter = Arbiter::new_system();
        let system = System::construct(sys_sender, arbiter.clone(), self.stop_on_panic);

        // system arbiter
        let arb = SystemArbiter::new(stop_tx, sys_receiver);

        let mut rt = self.build_rt().unwrap();
        rt.spawn(arb);

        // init system arbiter and run configuration method
        let _ = rt.block_on(lazy(move || {
            f();
            Ok::<_, ()>(())
        }));

        SystemRunner { rt, stop, system }
    }

    pub(crate) fn build_rt(&self) -> io::Result<Runtime> {
        // We need a reactor to receive events about IO objects from kernel
        let reactor = Reactor::new()?;
        let reactor_handle = reactor.handle();

        // Place a timer wheel on top of the reactor. If there are no timeouts to fire, it'll let the
        // reactor pick up some new external events.
        let timer = Timer::new_with_now(reactor, self.clock.clone());
        let timer_handle = timer.handle();

        // And now put a single-threaded executor on top of the timer. When there are no futures ready
        // to do something, it'll let the timer or the reactor to generate some new stimuli for the
        // futures to continue in their life.
        let executor = CurrentThread::new_with_park(timer);

        Ok(Runtime::new2(
            reactor_handle,
            timer_handle,
            self.clock.clone(),
            executor,
        ))
    }
}

/// Helper object that runs System's event loop
#[must_use = "SystemRunner must be run"]
#[derive(Debug)]
pub struct SystemRunner {
    rt: Runtime,
    stop: Receiver<i32>,
    system: System,
}

impl SystemRunner {
    /// This function will start event loop and will finish once the
    /// `System::stop()` function is called.
    pub fn run(self) -> i32 {
        let SystemRunner { mut rt, stop, .. } = self;

        // run loop
        let _ = rt.block_on(lazy(move || {
            Arbiter::run_system();
            Ok::<_, ()>(())
        }));
        let code = match rt.block_on(stop) {
            Ok(code) => code,
            Err(_) => 1,
        };
        Arbiter::stop_system();
        code
    }

    /// Execute a future and wait for result.
    pub fn block_on<F, I, E>(&mut self, fut: F) -> Result<I, E>
    where
        F: Future<Item = I, Error = E>,
    {
        let _ = self.rt.block_on(lazy(move || {
            Arbiter::run_system();
            Ok::<_, ()>(())
        }));
        let res = self.rt.block_on(fut);
        let _ = self.rt.block_on(lazy(move || {
            Arbiter::stop_system();
            Ok::<_, ()>(())
        }));
        res
    }
}
