//! A value that must never be printed, logged or written: the login token.

use std::fmt;

/// Holds the access token. It has no `Display`, `Clone` or serialisation, and
/// its `Debug` says nothing, so a stray `{:?}` or `dbg!` cannot leak it. The
/// only way out is [`Secret::expose`], which this crate calls in one place: the
/// `Authorization` header of the one request.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Secret {
        Secret(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_value() {
        let secret = Secret::new("sk-ant-oat01-very-secret".to_owned());
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(format!("{secret:#?}"), "[redacted]");
        assert_eq!(format!("{:?}", Some(&secret)), "Some([redacted])");
    }

    #[test]
    fn expose_returns_the_value() {
        assert_eq!(Secret::new("t".to_owned()).expose(), "t");
        assert!(Secret::new(String::new()).is_empty());
    }
}
