use std::cell::Cell;
use std::rc::Rc;

use futures::task::AtomicTask;

#[derive(Clone)]
/// Simple counter with ability to notify task on reaching specific number
///
/// Counter could be cloned, total ncount is shared across all clones.
pub struct Counter(Rc<CounterInner>);

struct CounterInner {
    count: Cell<usize>,
    max: usize,
    task: AtomicTask,
}

impl Counter {
    /// Create `Counter` instance and set max value.
    pub fn new(max: usize) -> Self {
        Counter(Rc::new(CounterInner {
            max,
            count: Cell::new(0),
            task: AtomicTask::new(),
        }))
    }

    pub fn get(&self) -> CounterGuard {
        CounterGuard::new(self.0.clone())
    }

    pub fn check(&self) -> bool {
        self.0.check()
    }

    pub fn total(&self) -> usize {
        self.0.count.get()
    }
}

pub struct CounterGuard(Rc<CounterInner>);

impl CounterGuard {
    fn new(inner: Rc<CounterInner>) -> Self {
        inner.inc();
        CounterGuard(inner)
    }
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

impl CounterInner {
    fn inc(&self) {
        let num = self.count.get() + 1;
        self.count.set(num);
        if num == self.max {
            self.task.register();
        }
    }

    fn dec(&self) {
        let num = self.count.get();
        self.count.set(num - 1);
        if num == self.max {
            self.task.notify();
        }
    }

    fn check(&self) -> bool {
        self.count.get() < self.max
    }
}
