//! A password being typed: never shown by `Debug`, overwritten with zeros when dropped. The
//! text field of iced keeps its own copy while it is on screen; that copy goes with the field.
//! Shared by the Wi-Fi password of the bar and the lock screen (lot D-25a).

use std::fmt;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(typed: String) -> Self {
        Self(typed)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Overwrites the password with zeros, then empties it.
    pub fn wipe(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
        // Keeps the writes: the buffer is read after them.
        std::hint::black_box(&bytes);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.wipe();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_never_printed() {
        let secret = Secret::new("not-a-real-password".into());
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(secret.as_str(), "not-a-real-password");
    }
}
