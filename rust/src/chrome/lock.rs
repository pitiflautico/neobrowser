//! Profile locks, and telling a stale one from a live one.
//!
//! Chrome refuses to start on a profile another instance holds. After a crash the lock
//! remains with no process behind it, and clearing it blindly would corrupt a profile that
//! a *running* Chrome still owns — so the holder's pid is checked for liveness first.

use std::path::Path;

/// Remove `Singleton*` from a profile when the process that created them is
/// gone.
///
/// `SingletonLock` is a symlink whose target is `hostname-pid`. If that pid is
/// still alive the lock is legitimate — a real Chrome owns this profile — and
/// nothing is removed, so we never yank the profile out from under a running
/// sibling. Only a genuinely orphaned lock is cleared.
pub(super) fn clear_stale_lock(profile_dir: &Path) {
    let Some(pid) = lock_pid(profile_dir) else {
        return; // no lock, or a format we don't recognize — leave it alone
    };
    if pid_alive(pid) {
        return; // a live Chrome owns this profile
    }
    for f in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let _ = std::fs::remove_file(profile_dir.join(f));
    }
}

/// The pid recorded in `SingletonLock`, whether or not it is still running.
/// The lock is a symlink whose target is `hostname-pid`.
fn lock_pid(profile_dir: &Path) -> Option<i32> {
    let target = std::fs::read_link(profile_dir.join("SingletonLock")).ok()?;
    target
        .to_string_lossy()
        .rsplit('-')
        .next()
        .and_then(|p| p.parse::<i32>().ok())
}

/// The pid of a **live** Chrome holding this profile, if any.
pub(super) fn lock_holder_pid(profile_dir: &Path) -> Option<i32> {
    lock_pid(profile_dir).filter(|pid| pid_alive(*pid))
}

/// The debug port a running Chrome published for this profile.
///
/// Chrome writes `DevToolsActivePort` into the user-data dir at startup: the
/// port on the first line, the browser's WS path on the second. Reading it lets
/// the "profile in use" error name the exact port to attach to instead of
/// telling the caller to go hunting for it.
pub(super) fn devtools_active_port(profile_dir: &Path) -> Option<u16> {
    let s = std::fs::read_to_string(profile_dir.join("DevToolsActivePort")).ok()?;
    s.lines().next()?.trim().parse::<u16>().ok()
}

/// Test hook: exercise the stale-lock rule without spawning Chrome.
#[doc(hidden)]
pub fn clear_stale_lock_for_test(profile_dir: &Path) {
    clear_stale_lock(profile_dir);
}

/// Public view of the profile lock, for `doctor --json`.
///
/// Returns the pid holding the profile, if any. Reported rather than cleared: doctor
/// diagnoses, it does not mutate the environment it is inspecting.
pub fn profile_lock_holder(profile_dir: &Path) -> Option<i32> {
    lock_holder_pid(profile_dir)
}

/// Does a process with this pid exist? Signal 0 checks for existence without
/// delivering anything.
fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        if pid <= 0 {
            return false;
        }
        // SAFETY: signal 0 is the documented "does this process exist" probe — it
        // delivers nothing and cannot affect the target. `pid` is checked positive above,
        // so this can never become the `kill(0, ..)` form that signals the whole process
        // group, nor the negative form that signals a group by id.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true // can't tell: assume alive and leave the lock in place
    }
}

pub(super) fn term(pid: u32) {
    #[cfg(unix)]
    // SAFETY: `pid` comes from `Child::id()` on a process this manager spawned, so it is
    // positive and ours. Positivity matters beyond type-correctness: `kill(0, ..)` would
    // signal our entire process group and a negative value would signal a group by id —
    // either would take down the caller's own process tree. A `u32` from `Child::id()`
    // cannot be zero or negative, which is what makes this call sound.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = pid; // handled via Child::start_kill on the Windows path
    }
}
