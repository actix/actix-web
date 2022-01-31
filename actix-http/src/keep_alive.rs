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
        !matches!(self, Self::Disabled)
    }

    #[allow(unused)] // used with `http2` feature flag
    pub(crate) fn duration(&self) -> Option<Duration> {
        match self {
            KeepAlive::Timeout(dur) => Some(*dur),
            _ => None,
        }
    }

    /// Map zero duration to disabled.
    pub(crate) fn normalize(self) -> KeepAlive {
        match self {
            KeepAlive::Timeout(Duration::ZERO) => KeepAlive::Disabled,
            ka => ka,
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
        KeepAlive::Timeout(dur).normalize()
    }
}

impl From<Option<Duration>> for KeepAlive {
    fn from(ka_dur: Option<Duration>) -> Self {
        match ka_dur {
            Some(dur) => KeepAlive::from(dur),
            None => KeepAlive::Disabled,
        }
        .normalize()
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

        let test: KeepAlive = Some(Duration::from_secs(0)).into();
        assert_eq!(test, KeepAlive::Disabled);

        let test: KeepAlive = None.into();
        assert_eq!(test, KeepAlive::Disabled);
    }
}
