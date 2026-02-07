use std::{cell::Cell, marker::PhantomData, rc::Rc, task};

use local_waker::LocalWaker;

/// Counter. It tracks of number of clones of payloads and give access to payload only to top most.
///
/// - When dropped, parent task is awakened. This is to support the case where `Field` is dropped in
///   a separate task than `Multipart`.
/// - Assumes that parent owners don't move to different tasks; only the top-most is allowed to.
/// - If dropped and is not top most owner, is_clean flag is set to false.
#[derive(Debug)]
pub(crate) struct Safety {
    task: LocalWaker,
    level: usize,
    payload: Rc<PhantomData<bool>>,
    clean: Rc<Cell<bool>>,
}

impl Safety {
    pub(crate) fn new() -> Safety {
        let payload = Rc::new(PhantomData);
        Safety {
            task: LocalWaker::new(),
            level: Rc::strong_count(&payload),
            clean: Rc::new(Cell::new(true)),
            payload,
        }
    }

    pub(crate) fn current(&self) -> bool {
        Rc::strong_count(&self.payload) == self.level && self.clean.get()
    }

    pub(crate) fn is_clean(&self) -> bool {
        self.clean.get()
    }

    pub(crate) fn clone(&self, cx: &task::Context<'_>) -> Safety {
        let payload = Rc::clone(&self.payload);
        let s = Safety {
            task: LocalWaker::new(),
            level: Rc::strong_count(&payload),
            clean: self.clean.clone(),
            payload,
        };
        s.task.register(cx.waker());
        s
    }
}

impl Drop for Safety {
    fn drop(&mut self) {
        if Rc::strong_count(&self.payload) != self.level {
            // Multipart dropped leaving a Field
            self.clean.set(false);
        }

        self.task.wake();
    }
}
