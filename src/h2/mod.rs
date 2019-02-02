use std::fmt;

mod service;

/// H1 service response type
pub enum H2ServiceResult<T> {
    Disconnected,
    Shutdown(T),
}

impl<T: fmt::Debug> fmt::Debug for H2ServiceResult<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            H2ServiceResult::Disconnected => write!(f, "H2ServiceResult::Disconnected"),
            H2ServiceResult::Shutdown(ref v) => {
                write!(f, "H2ServiceResult::Shutdown({:?})", v)
            }
        }
    }
}
