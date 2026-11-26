//! The password check, by PAM only (ADR 0023): the lock never checks a password itself and
//! never keeps one. PAM is opened at run time (`libpam.so.0`, through `libloading`), with the
//! service `sherlockos-lock` (`/etc/pam.d/sherlockos-lock`), for the user of the session only.
//! The check blocks (PAM waits on purpose after a wrong password): it runs on a thread of its
//! own. The password goes to PAM through the conversation function, in a copy PAM frees; ours
//! is overwritten with zeros as soon as it is handed over, and never logged.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;

use system_services::secret::Secret;
use zeroize::Zeroize;

/// The PAM service of the lock: `/etc/pam.d/sherlockos-lock`.
pub const SERVICE: &str = "sherlockos-lock";
/// The library, as the system installs it (no development package needed).
const LIBRARY: &str = "libpam.so.0";

// Values of <security/_pam_types.h> (Linux-PAM).
const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_BUF_ERR: c_int = 5;
const PAM_AUTH_ERR: c_int = 7;
const PAM_USER_UNKNOWN: c_int = 10;
const PAM_MAXTRIES: c_int = 11;
const PAM_CONV_ERR: c_int = 19;
const PAM_DISALLOW_NULL_AUTHTOK: c_int = 0x1;

#[repr(C)]
pub struct Message {
    pub style: c_int,
    pub text: *const c_char,
}

#[repr(C)]
pub struct Response {
    pub resp: *mut c_char,
    pub retcode: c_int,
}

type ConvFn =
    unsafe extern "C" fn(c_int, *mut *const Message, *mut *mut Response, *mut c_void) -> c_int;

#[repr(C)]
struct Conv {
    conv: ConvFn,
    appdata: *mut c_void,
}

/// Why a check failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The password is wrong (or PAM refused it).
    #[error("authentication refused")]
    Refused,
    /// Too many tries: PAM says so (`PAM_MAXTRIES`). The lock screen runs as the user, who cannot
    /// write the counters of pam_faillock: only the delay of pam_faildelay slows the tries.
    #[error("too many tries")]
    TooManyTries,
    /// PAM itself could not run: no library, no service, an error of the system.
    #[error("PAM unavailable: {0}")]
    Unavailable(String),
}

/// What checks a password: PAM, or a fake one in the tests.
pub trait Authenticator: Send + Sync + 'static {
    fn check(&self, user: &str, password: Secret) -> Result<(), AuthError>;
}

/// The system's PAM, with the service `SERVICE`.
#[derive(Debug, Clone, Default)]
pub struct Pam;

/// What the conversation answers: the user for a visible prompt, the password for a hidden
/// one, given once only.
pub struct Conversation {
    user: CString,
    password: Option<Secret>,
    /// The messages PAM showed.
    pub messages: Vec<String>,
}

impl Conversation {
    pub fn new(user: &str, password: Secret) -> Option<Self> {
        Some(Self {
            user: CString::new(user).ok()?,
            password: Some(password),
            messages: Vec::new(),
        })
    }

    /// The answer to one message: `None` for a message without answer, `Err` for a prompt
    /// that cannot be answered.
    fn answer(&mut self, style: c_int, text: &str) -> Result<Option<CString>, ()> {
        match style {
            PAM_PROMPT_ECHO_OFF => {
                let password = self.password.take().ok_or(())?;
                CString::new(password.as_str()).map(Some).map_err(|_| ())
            }
            PAM_PROMPT_ECHO_ON => Ok(Some(self.user.clone())),
            PAM_ERROR_MSG | PAM_TEXT_INFO => {
                self.messages.push(text.to_owned());
                Ok(None)
            }
            _ => Err(()),
        }
    }
}

/// Overwrites a C string with zeros (volatile writes).
fn wipe(text: CString) {
    text.into_bytes_with_nul().zeroize();
}

/// The conversation function PAM calls: one answer per message, allocated with `malloc` for
/// PAM to free.
///
/// # Safety
///
/// `messages` points to `count` valid messages (Linux-PAM's array of pointers), `responses`
/// is writable, and `data` is the `Conversation` given to `pam_start`, alive and not borrowed
/// elsewhere during the call.
pub unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const Message,
    responses: *mut *mut Response,
    data: *mut c_void,
) -> c_int {
    let Ok(n) = usize::try_from(count) else {
        return PAM_CONV_ERR;
    };
    if n == 0 || messages.is_null() || responses.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: `data` is the conversation given to `pam_start` (contract of this function).
    let conversation = unsafe { &mut *data.cast::<Conversation>() };
    // SAFETY: calloc of `n` responses, checked for null below; PAM frees it.
    let answers = unsafe { libc::calloc(n, std::mem::size_of::<Response>()) }.cast::<Response>();
    if answers.is_null() {
        return PAM_BUF_ERR;
    }
    for i in 0..n {
        // SAFETY: `messages` holds `n` pointers to valid messages (contract of this function).
        let message = unsafe { &**messages.add(i) };
        let text = match message.text.is_null() {
            true => String::new(),
            // SAFETY: PAM gives a NUL-terminated string that lives during the call.
            false => unsafe { CStr::from_ptr(message.text) }
                .to_string_lossy()
                .into_owned(),
        };
        let answer = match conversation.answer(message.style, &text) {
            Ok(answer) => answer,
            Err(()) => {
                // SAFETY: `answers` holds `n` responses, the first `i` filled by us.
                unsafe { free_responses(answers, n) };
                return PAM_CONV_ERR;
            }
        };
        if let Some(answer) = answer {
            // SAFETY: a NUL-terminated string, copied with malloc for PAM to free.
            let copy = unsafe { libc::strdup(answer.as_ptr()) };
            wipe(answer);
            if copy.is_null() {
                // SAFETY: as above.
                unsafe { free_responses(answers, n) };
                return PAM_BUF_ERR;
            }
            // SAFETY: `i < n`, inside the array allocated above.
            unsafe { (*answers.add(i)).resp = copy };
        }
    }
    // SAFETY: `responses` is writable (contract of this function).
    unsafe { *responses = answers };
    PAM_SUCCESS
}

/// Frees answers, their texts overwritten with zeros first.
///
/// # Safety
///
/// `answers` was allocated by `converse` for `n` responses.
pub unsafe fn free_responses(answers: *mut Response, n: usize) {
    for i in 0..n {
        // SAFETY: inside the array (contract of this function).
        let resp = unsafe { (*answers.add(i)).resp };
        if !resp.is_null() {
            // SAFETY: a string from strdup, NUL-terminated, overwritten then freed.
            unsafe {
                let len = libc::strlen(resp);
                ptr::write_bytes(resp, 0, len);
                libc::free(resp.cast());
            }
        }
    }
    // SAFETY: allocated by calloc in `converse`.
    unsafe { libc::free(answers.cast()) };
}

type StartFn =
    unsafe extern "C" fn(*const c_char, *const c_char, *const Conv, *mut *mut c_void) -> c_int;
type StepFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;

impl Authenticator for Pam {
    fn check(&self, user: &str, password: Secret) -> Result<(), AuthError> {
        let unavailable = |what: &str| AuthError::Unavailable(what.to_owned());
        let mut conversation =
            Conversation::new(user, password).ok_or_else(|| unavailable("user name"))?;
        let service = CString::new(SERVICE).map_err(|_| unavailable("service"))?;
        // SAFETY: loading the system's libpam runs its initialisers, which only set up the
        // library itself.
        let library = unsafe { libloading::Library::new(LIBRARY) }
            .map_err(|err| AuthError::Unavailable(err.to_string()))?;
        // SAFETY: the symbols have the types of <security/pam_appl.h>.
        let (start, authenticate, account, end) = unsafe {
            let start = library.get::<StartFn>(b"pam_start\0");
            let authenticate = library.get::<StepFn>(b"pam_authenticate\0");
            let account = library.get::<StepFn>(b"pam_acct_mgmt\0");
            let end = library.get::<StepFn>(b"pam_end\0");
            match (start, authenticate, account, end) {
                (Ok(s), Ok(a), Ok(m), Ok(e)) => (*s, *a, *m, *e),
                _ => return Err(unavailable("symbols")),
            }
        };
        let conv = Conv {
            conv: converse,
            appdata: (&raw mut conversation).cast(),
        };
        let mut handle = ptr::null_mut();
        // SAFETY: valid strings, a conversation that outlives the handle (pam_end below), and a
        // place for the handle.
        let status = unsafe {
            start(
                service.as_ptr(),
                conversation.user.as_ptr(),
                &conv,
                &mut handle,
            )
        };
        if status != PAM_SUCCESS || handle.is_null() {
            return Err(AuthError::Unavailable(format!("pam_start: {status}")));
        }
        // SAFETY: a handle from pam_start, used on this thread only, ended once.
        let status = unsafe {
            let mut status = authenticate(handle, PAM_DISALLOW_NULL_AUTHTOK);
            if status == PAM_SUCCESS {
                status = account(handle, 0);
            }
            end(handle, status);
            status
        };
        drop(conversation);
        match status {
            PAM_SUCCESS => Ok(()),
            PAM_AUTH_ERR | PAM_USER_UNKNOWN => Err(AuthError::Refused),
            PAM_MAXTRIES => Err(AuthError::TooManyTries),
            other => Err(AuthError::Unavailable(format!("PAM status {other}"))),
        }
    }
}

/// The user of this session, from the password database (not from `$USER`).
pub fn session_user() -> Option<String> {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let mut buffer = vec![0 as c_char; 4096];
    // SAFETY: an all-zero passwd is a valid value for getpwuid_r to fill.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = ptr::null_mut();
    // SAFETY: the buffer and the entry live during the call; `found` points to `entry` or is
    // null.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated name inside `buffer`, alive here.
    let name = unsafe { CStr::from_ptr(entry.pw_name) };
    name.to_str().ok().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A conversation as PAM leads it: `messages`, then the answers read back and freed.
    fn converse_with(
        conversation: &mut Conversation,
        messages: &[(c_int, &str)],
    ) -> Result<Vec<Option<String>>, c_int> {
        let texts: Vec<CString> = messages
            .iter()
            .map(|(_, t)| CString::new(*t).unwrap())
            .collect();
        let owned: Vec<Message> = messages
            .iter()
            .zip(&texts)
            .map(|((style, _), text)| Message {
                style: *style,
                text: text.as_ptr(),
            })
            .collect();
        let mut pointers: Vec<*const Message> = owned.iter().map(|m| m as *const Message).collect();
        let mut answers = ptr::null_mut();
        // SAFETY: valid messages, a place for the answers, a live conversation.
        let status = unsafe {
            converse(
                messages.len() as c_int,
                pointers.as_mut_ptr(),
                &mut answers,
                (conversation as *mut Conversation).cast(),
            )
        };
        if status != PAM_SUCCESS {
            return Err(status);
        }
        let read = (0..messages.len())
            .map(|i| {
                // SAFETY: `converse` filled `messages.len()` answers.
                let resp = unsafe { (*answers.add(i)).resp };
                (!resp.is_null()).then(|| {
                    // SAFETY: a string from strdup, not null.
                    unsafe { CStr::from_ptr(resp) }
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .collect();
        // SAFETY: allocated by `converse` for that many answers.
        unsafe { free_responses(answers, messages.len()) };
        Ok(read)
    }

    // A fake password, never the author's.
    const FAKE: &str = "correct-horse-test";

    #[test]
    fn the_password_answers_the_hidden_prompt_once() {
        let mut c = Conversation::new("alice", Secret::new(FAKE.into())).unwrap();
        let read = converse_with(
            &mut c,
            &[
                (PAM_PROMPT_ECHO_ON, "login:"),
                (PAM_PROMPT_ECHO_OFF, "Password:"),
            ],
        );
        assert_eq!(
            read.unwrap(),
            [Some("alice".to_owned()), Some(FAKE.to_owned())]
        );
        // A second hidden prompt has nothing to answer: the conversation fails.
        assert_eq!(
            converse_with(&mut c, &[(PAM_PROMPT_ECHO_OFF, "Password:")]),
            Err(PAM_CONV_ERR)
        );
    }

    #[test]
    fn messages_are_kept_without_answer() {
        let mut c = Conversation::new("alice", Secret::new(FAKE.into())).unwrap();
        let read = converse_with(
            &mut c,
            &[
                (PAM_ERROR_MSG, "The account is locked"),
                (PAM_TEXT_INFO, "hello"),
            ],
        );
        assert_eq!(read.unwrap(), [None, None]);
        assert_eq!(c.messages, ["The account is locked", "hello"]);
        assert_eq!(converse_with(&mut c, &[(42, "?")]), Err(PAM_CONV_ERR));
    }

    #[test]
    fn the_session_user_is_read_from_the_password_database() {
        assert!(session_user().is_some_and(|name| !name.is_empty()));
    }
}
