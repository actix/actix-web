use std::time::Duration;

/// Connection keep-alive config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAlive {
    /// Keep-alive duration.
    ///
    /// `KeepAlive::Timeout(Duration::ZERO)` is mapped to `KeepAlive::Disabled`.
    Timeout(Duration),

    /// Rely on OS to shutdown TCP connection.
    ///
    /// Some defaults can be very long, check your OS documentation.
    Os,

    /// Keep-alive is disabled.
    ///
    /// Connections will be closed immediately.
    Disabled,
}

impl KeepAlive {
    pub(crate) fn enabled(&self) -> bool {
        matches!(self, Self::Timeout(_) | Self::Os)
    }

    pub(crate) fn duration(&self) -> Option<Duration> {
        match self {
            KeepAlive::Timeout(dur) => Some(*dur),
            _ => None,
        }
    }
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self::Timeout(Duration::from_secs(5))
    }
}

impl From<Duration> for KeepAlive {
    fn from(dur: Duration) -> Self {
        KeepAlive::Timeout(dur)
    }
}

impl From<Option<Duration>> for KeepAlive {
    fn from(ka_dur: Option<Duration>) -> Self {
        match ka_dur {
            Some(Duration::ZERO) => KeepAlive::Disabled,
            Some(dur) => KeepAlive::Timeout(dur),
            None => KeepAlive::Disabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_impls() {
        let test: KeepAlive = Duration::from_secs(1).into();
        assert_eq!(test, KeepAlive::Timeout(Duration::from_secs(1)));

        let test: KeepAlive = Duration::from_secs(0).into();
        assert_eq!(test, KeepAlive::Disabled);

        let test: KeepAlive = None.into();
        assert_eq!(test, KeepAlive::Disabled);
    }
}
