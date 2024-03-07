/// Test Module for checking the drop state of certain async tasks that are spawned
/// with `actix_rt::spawn`
///
/// The target task must explicitly generate `NotifyOnDrop` when spawn the task
use std::cell::RefCell;

thread_local! {
    static NOTIFY_DROPPED: RefCell<Option<bool>> = const { RefCell::new(None) };
}

/// Check if the spawned task is dropped.
///
/// # Panics
/// Panics when there was no `NotifyOnDrop` instance on current thread.
pub(crate) fn is_dropped() -> bool {
    NOTIFY_DROPPED.with(|bool| {
        bool.borrow()
            .expect("No NotifyOnDrop existed on current thread")
    })
}

pub(crate) struct NotifyOnDrop;

impl NotifyOnDrop {
    /// # Panics
    /// Panics hen construct multiple instances on any given thread.
    pub(crate) fn new() -> Self {
        NOTIFY_DROPPED.with(|bool| {
            let mut bool = bool.borrow_mut();
            if bool.is_some() {
                panic!("NotifyOnDrop existed on current thread");
            } else {
                *bool = Some(false);
            }
        });

        NotifyOnDrop
    }
}

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        NOTIFY_DROPPED.with(|bool| {
            if let Some(b) = bool.borrow_mut().as_mut() {
                *b = true;
            }
        });
    }
}
