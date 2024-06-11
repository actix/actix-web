use std::{fmt, future::Future, pin::Pin, task::Context};

use actix_rt::time::{Instant, Sleep};
use tracing::trace;

#[derive(Debug)]
pub(super) enum TimerState {
    Disabled,
    Inactive,
    Active { timer: Pin<Box<Sleep>> },
}

impl TimerState {
    pub(super) fn new(enabled: bool) -> Self {
        if enabled {
            Self::Inactive
        } else {
            Self::Disabled
        }
    }

    pub(super) fn is_enabled(&self) -> bool {
        matches!(self, Self::Active { .. } | Self::Inactive)
    }

    pub(super) fn set(&mut self, timer: Sleep, line: u32) {
        if matches!(self, Self::Disabled) {
            trace!("setting disabled timer from line {}", line);
        }

        *self = Self::Active {
            timer: Box::pin(timer),
        };
    }

    pub(super) fn set_and_init(&mut self, cx: &mut Context<'_>, timer: Sleep, line: u32) {
        self.set(timer, line);
        self.init(cx);
    }

    pub(super) fn clear(&mut self, line: u32) {
        if matches!(self, Self::Disabled) {
            trace!("trying to clear a disabled timer from line {}", line);
        }

        if matches!(self, Self::Inactive) {
            trace!("trying to clear an inactive timer from line {}", line);
        }

        *self = Self::Inactive;
    }

    pub(super) fn init(&mut self, cx: &mut Context<'_>) {
        if let TimerState::Active { timer } = self {
            let _ = timer.as_mut().poll(cx);
        }
    }
}

impl fmt::Display for TimerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimerState::Disabled => f.write_str("timer is disabled"),
            TimerState::Inactive => f.write_str("timer is inactive"),
            TimerState::Active { timer } => {
                let deadline = timer.deadline();
                let now = Instant::now();

                if deadline < now {
                    f.write_str("timer is active and has reached deadline")
                } else {
                    write!(
                        f,
                        "timer is active and due to expire in {} milliseconds",
                        ((deadline - now).as_secs_f32() * 1000.0)
                    )
                }
            }
        }
    }
}
