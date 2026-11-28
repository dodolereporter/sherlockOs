//! The failed tries counted (lot D-25n), since pam_faillock cannot count them for the lock
//! screen: it runs as the user, and pam_faillock keeps its counters in `/var/run/faillock`,
//! which only root may write (its `preauth` and `authfail` cannot open them and let the try
//! through); and Debian's `common-auth` does not call it. See "Tries" in `docs/lock.md`.
//!
//! `FREE_TRIES` wrong passwords in a row, then each new one makes the next try wait, twice as
//! long each time, from `FIRST_WAIT` to `LONGEST_WAIT` at most: never for good, the user is never
//! shut out of their session. While it waits, a try is answered `TooManyTries` at once, without
//! reaching PAM, and the password is wiped. The right password starts the count again. This is a
//! delay, not a check: PAM alone says whether a password is right (ADR 0023). Nothing is stored:
//! the count lives as long as the lock program.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use system_services::secret::Secret;

use crate::pam::{AuthError, Authenticator};

/// Wrong passwords in a row before the tries are slowed.
pub const FREE_TRIES: u32 = 5;
/// The wait after the first wrong password past `FREE_TRIES`, doubled at each one after.
pub const FIRST_WAIT: Duration = Duration::from_secs(30);
/// The longest wait: the user always gets another try.
pub const LONGEST_WAIT: Duration = Duration::from_secs(5 * 60);

/// How long the next try waits after `failures` wrong passwords in a row; `None` while they
/// are free.
pub fn wait_after(failures: u32) -> Option<Duration> {
    let beyond = failures.checked_sub(FREE_TRIES)?;
    let doubling = 1u32.checked_shl(beyond).unwrap_or(u32::MAX);
    Some(FIRST_WAIT.saturating_mul(doubling).min(LONGEST_WAIT))
}

#[derive(Debug, Default)]
struct Tries {
    failures: u32,
    until: Option<Instant>,
}

/// An authenticator whose wrong passwords are counted and slowed.
pub struct Throttled<A> {
    inner: A,
    tries: Mutex<Tries>,
    clock: fn() -> Instant,
}

impl<A: Authenticator> Throttled<A> {
    pub fn new(inner: A) -> Self {
        Self::with_clock(inner, Instant::now)
    }

    /// With the clock `clock` (the tests).
    pub fn with_clock(inner: A, clock: fn() -> Instant) -> Self {
        Self {
            inner,
            tries: Mutex::new(Tries::default()),
            clock,
        }
    }

    fn tries(&self) -> std::sync::MutexGuard<'_, Tries> {
        self.tries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<A: Authenticator> Authenticator for Throttled<A> {
    fn check(&self, user: &str, password: Secret) -> Result<(), AuthError> {
        if self
            .tries()
            .until
            .is_some_and(|until| (self.clock)() < until)
        {
            // Wiped on drop, never handed to PAM.
            drop(password);
            tracing::warn!("too many wrong passwords: this try waits");
            return Err(AuthError::TooManyTries);
        }
        let result = self.inner.check(user, password);
        let mut tries = self.tries();
        match &result {
            Ok(()) => *tries = Tries::default(),
            Err(AuthError::Refused | AuthError::TooManyTries) => {
                tries.failures = tries.failures.saturating_add(1);
                tries.until = wait_after(tries.failures).map(|wait| (self.clock)() + wait);
                if let Some(wait) = wait_after(tries.failures) {
                    tracing::warn!(
                        failures = tries.failures,
                        wait_s = wait.as_secs(),
                        "wrong passwords in a row: the next try waits"
                    );
                }
            }
            // PAM did not answer: nothing to count.
            Err(AuthError::Unavailable(_) | AuthError::Busy) => {}
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{LazyLock, Mutex as StdMutex};

    /// A clock the test moves, in seconds from a fixed start.
    static START: LazyLock<Instant> = LazyLock::new(Instant::now);
    static SECONDS: AtomicU64 = AtomicU64::new(0);
    fn clock() -> Instant {
        *START + Duration::from_secs(SECONDS.load(Ordering::SeqCst))
    }

    /// Accepts `right` only; counts the passwords it was handed.
    struct Fake(StdMutex<u32>);

    impl Authenticator for Fake {
        fn check(&self, _: &str, password: Secret) -> Result<(), AuthError> {
            *self.0.lock().unwrap() += 1;
            match password.as_str() == "right" {
                true => Ok(()),
                false => Err(AuthError::Refused),
            }
        }
    }

    fn try_with(lock: &Throttled<Fake>, password: &str) -> Result<(), AuthError> {
        lock.check("sam", Secret::new(password.into()))
    }

    #[test]
    fn the_waits_double_and_never_lock_out_for_good() {
        assert_eq!(wait_after(FREE_TRIES - 1), None);
        assert_eq!(wait_after(FREE_TRIES), Some(FIRST_WAIT));
        assert_eq!(wait_after(FREE_TRIES + 1), Some(FIRST_WAIT * 2));
        assert_eq!(wait_after(FREE_TRIES + 20), Some(LONGEST_WAIT));
        assert_eq!(wait_after(u32::MAX), Some(LONGEST_WAIT));
    }

    #[test]
    fn wrong_passwords_are_slowed_then_the_right_one_starts_again() {
        let lock = Throttled::with_clock(Fake(StdMutex::new(0)), clock);
        for _ in 0..FREE_TRIES {
            assert_eq!(try_with(&lock, "wrong"), Err(AuthError::Refused));
        }
        // Now it waits: even the right password is not handed to PAM.
        assert_eq!(try_with(&lock, "right"), Err(AuthError::TooManyTries));
        assert_eq!(*lock.inner.0.lock().unwrap(), FREE_TRIES);
        SECONDS.fetch_add(FIRST_WAIT.as_secs(), Ordering::SeqCst);
        // Its turn: a wrong one waits twice as long.
        assert_eq!(try_with(&lock, "wrong"), Err(AuthError::Refused));
        SECONDS.fetch_add(FIRST_WAIT.as_secs(), Ordering::SeqCst);
        assert_eq!(try_with(&lock, "right"), Err(AuthError::TooManyTries));
        SECONDS.fetch_add(FIRST_WAIT.as_secs(), Ordering::SeqCst);
        assert_eq!(try_with(&lock, "right"), Ok(()));
        // The count starts again.
        assert_eq!(try_with(&lock, "wrong"), Err(AuthError::Refused));
        assert_eq!(try_with(&lock, "right"), Ok(()));
    }
}
