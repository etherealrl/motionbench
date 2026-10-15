use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

/// how an analysis ended without a result. cancelling is its own variant, so nothing has to
/// recognise it by its wording
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str("analysis cancelled"),
            Error::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Error::Failed(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Error::Failed(message.into())
    }
}

/// the one place a cancel request becomes an error, so every check in the analysis stops the
/// same way
pub fn check(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// the same check for readers that only speak `io::Error`, the variant rides inside it and
/// comes back out as `Cancelled` rather than as text
pub fn check_io(cancel: &AtomicBool) -> std::io::Result<()> {
    check(cancel).map_err(std::io::Error::other)
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        match error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Error>())
        {
            Some(Error::Cancelled) => Error::Cancelled,
            _ => Error::Failed(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
