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
    capacity: usize,
    task: AtomicTask,
}

impl Counter {
    /// Create `Counter` instance and set max value.
    pub fn new(capacity: usize) -> Self {
        Counter(Rc::new(CounterInner {
            capacity,
            count: Cell::new(0),
            task: AtomicTask::new(),
        }))
    }

    pub fn get(&self) -> CounterGuard {
        CounterGuard::new(self.0.clone())
    }

    /// Check if counter is not at capacity
    pub fn available(&self) -> bool {
        self.0.available()
    }

    /// Get total number of acquired counts
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
        if num == self.capacity {
            self.task.register();
        }
    }

    fn dec(&self) {
        let num = self.count.get();
        self.count.set(num - 1);
        if num == self.capacity {
            self.task.notify();
        }
    }

    fn available(&self) -> bool {
        self.count.get() < self.capacity
    }
}
