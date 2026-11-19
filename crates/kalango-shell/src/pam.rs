//! Password check for the lock screen through PAM (the same path as the login screen): service `kalango-bloqueio`
//! (installed at /etc/pam.d), or `login` while it is not installed. Blocking: call it off the UI thread (PAM waits
//! a moment after a wrong password on purpose). `KALANGO_PAM_DIR` points PAM at another configuration folder
//! (`pam_start_confdir`): the session test uses one with a test password, never the person's.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

#[link(name = ":libpam.so.0")]
extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const PamConv, pamh: *mut *mut c_void) -> c_int;
    fn pam_start_confdir(service: *const c_char, user: *const c_char, conv: *const PamConv, confdir: *const c_char, pamh: *mut *mut c_void) -> c_int;
    fn pam_authenticate(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_setcred(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut c_void, status: c_int) -> c_int;
}

extern "C" {
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn strdup(s: *const c_char) -> *mut c_char;
}

/// Answers every hidden prompt with the password (and visible ones with nothing).
extern "C" fn conversation(n: c_int, msgs: *mut *const PamMessage, resp: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    if n <= 0 || msgs.is_null() || resp.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // PAM frees the array and each answer with free(): they come from calloc/strdup
    let out = unsafe { calloc(n as usize, std::mem::size_of::<PamResponse>()) } as *mut PamResponse;
    if out.is_null() {
        return PAM_BUF_ERR;
    }
    let password = data as *const c_char;
    for i in 0..n as usize {
        let style = unsafe { (**msgs.add(i)).msg_style };
        let r = unsafe { &mut *out.add(i) };
        r.resp_retcode = 0;
        r.resp = match style {
            PAM_PROMPT_ECHO_OFF => unsafe { strdup(password) },
            PAM_PROMPT_ECHO_ON => unsafe { strdup(c"".as_ptr()) },
            _ => std::ptr::null_mut(),
        };
    }
    unsafe { *resp = out };
    PAM_SUCCESS
}

fn service() -> &'static CStr {
    if std::path::Path::new("/etc/pam.d/kalango-bloqueio").is_file() || std::env::var_os("KALANGO_PAM_DIR").is_some() {
        c"kalango-bloqueio"
    } else {
        c"login"
    }
}

/// Is this the password of `user`?
pub fn check(user: &str, password: &str) -> bool {
    let (Ok(user), Ok(pw)) = (CString::new(user), CString::new(password)) else { return false };
    let conv = PamConv { conv: conversation, appdata_ptr: pw.as_ptr() as *mut c_void };
    let mut h: *mut c_void = std::ptr::null_mut();
    let started = match std::env::var("KALANGO_PAM_DIR").ok().and_then(|d| CString::new(d).ok()) {
        Some(dir) => unsafe { pam_start_confdir(service().as_ptr(), user.as_ptr(), &conv, dir.as_ptr(), &mut h) },
        None => unsafe { pam_start(service().as_ptr(), user.as_ptr(), &conv, &mut h) },
    };
    if started != PAM_SUCCESS || h.is_null() {
        return false;
    }
    let mut r = unsafe { pam_authenticate(h, 0) };
    if r == PAM_SUCCESS {
        r = unsafe { pam_acct_mgmt(h, 0) };
    }
    if r == PAM_SUCCESS {
        // refresh credentials (Kerberos tickets, keyrings) as other lock screens do
        unsafe { pam_setcred(h, 0x0008) }; // PAM_REINITIALIZE_CRED
    }
    unsafe { pam_end(h, r) };
    r == PAM_SUCCESS
}

#[cfg(test)]
pub(crate) mod tests {
    /// Writes an executable script through a child (`cat`), so this process never holds it open for writing.
    pub(crate) fn escrever_por_filho(p: &std::path::Path, corpo: &str) {
        use std::io::Write;
        let mut c = std::process::Command::new("sh").args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"]).arg(p)
            .stdin(std::process::Stdio::piped()).spawn().unwrap();
        c.stdin.take().unwrap().write_all(corpo.as_bytes()).unwrap();
        assert!(c.wait().unwrap().success());
    }

    /// A PAM configuration of its own (pam_exec checks a test password): the person's password is never used.
    #[test]
    fn checks_a_password_through_pam() {
        let dir = std::env::temp_dir().join(format!("kalango-pam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("confere.sh");
        // one read, not until EOF: pam_exec's pipe is not close-on-exec, and a child another test spawns meanwhile can
        // hold its write end open (then `read` waited for that child to end). Written by a child process, so no fork
        // of another test inherits the script open for writing ("text file busy").
        escrever_por_filho(&script, "#!/bin/sh\nsenha=$(dd bs=256 count=1 2>/dev/null | tr -d '\\000')\n[ \"$senha\" = senha-de-teste ]\n");
        std::fs::write(dir.join("kalango-bloqueio"), format!("auth required pam_exec.so expose_authtok quiet {}\naccount required pam_permit.so\n", script.display())).unwrap();
        std::env::set_var("KALANGO_PAM_DIR", &dir);
        let user = gtk::glib::user_name().to_string_lossy().into_owned();
        assert!(super::check(&user, "senha-de-teste"));
        assert!(!super::check(&user, "errada"));
        std::env::remove_var("KALANGO_PAM_DIR");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
