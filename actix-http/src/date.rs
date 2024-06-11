use std::{
    cell::Cell,
    fmt::{self, Write},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use actix_rt::{task::JoinHandle, time::interval};

/// "Thu, 01 Jan 1970 00:00:00 GMT".len()
pub(crate) const DATE_VALUE_LENGTH: usize = 29;

#[derive(Clone, Copy)]
pub(crate) struct Date {
    pub(crate) bytes: [u8; DATE_VALUE_LENGTH],
    pos: usize,
}

impl Date {
    fn new() -> Date {
        let mut date = Date {
            bytes: [0; DATE_VALUE_LENGTH],
            pos: 0,
        };
        date.update();
        date
    }

    fn update(&mut self) {
        self.pos = 0;
        write!(self, "{}", httpdate::HttpDate::from(SystemTime::now())).unwrap();
    }
}

impl fmt::Write for Date {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let len = s.len();
        self.bytes[self.pos..self.pos + len].copy_from_slice(s.as_bytes());
        self.pos += len;
        Ok(())
    }
}

/// Service for update Date and Instant periodically at 500 millis interval.
pub(crate) struct DateService {
    current: Rc<Cell<(Date, Instant)>>,
    handle: JoinHandle<()>,
}

impl DateService {
    pub(crate) fn new() -> Self {
        // shared date and timer for DateService and update async task.
        let current = Rc::new(Cell::new((Date::new(), Instant::now())));
        let current_clone = Rc::clone(&current);
        // spawn an async task sleep for 500 millis and update current date/timer in a loop.
        // handle is used to stop the task on DateService drop.
        let handle = actix_rt::spawn(async move {
            #[cfg(test)]
            let _notify = crate::notify_on_drop::NotifyOnDrop::new();

            let mut interval = interval(Duration::from_millis(500));
            loop {
                let now = interval.tick().await;
                let date = Date::new();
                current_clone.set((date, now.into_std()));
            }
        });

        DateService { current, handle }
    }

    pub(crate) fn now(&self) -> Instant {
        self.current.get().1
    }

    pub(crate) fn with_date<F: FnMut(&Date)>(&self, mut f: F) {
        f(&self.current.get().0);
    }
}

impl fmt::Debug for DateService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DateService").finish_non_exhaustive()
    }
}

impl Drop for DateService {
    fn drop(&mut self) {
        // stop the timer update async task on drop.
        self.handle.abort();
    }
}
