//! Deciding whether Chrome's renderer sandbox can be enabled, and refusing to lie about it.
//!
//! The sandbox is the single most important security boundary this tool depends on, and it
//! is also the one most easily switched off by accident: `--no-sandbox` makes a stubborn
//! container work, so it spreads through documentation and stays forever. So the decision is
//! made here, once, and it fails closed — an unavailable sandbox is an error unless someone
//! opted in explicitly, and the opt-in is recorded so `status` can report the truth.

//! Tier 0: Chrome process manager.
//!
//! Port of the Python `chrome_process.py`. Design invariants kept:
//! - No shared PID file that could kill sibling processes.
//! - A `ChromeProcess` owns exactly the child it spawned and only ever kills that.
//! - `health_check()` requires BOTH the process alive AND the debug port responding,
//!   which prevents handing out a zombie ("GhostChrome").
//!
//! Improvements over the Python original:
//! - The spawned child is owned, so `Drop` reaps it — no orphan Chromes if the
//!   manager is dropped without an explicit `kill()`.
//! - `kill()` sends SIGTERM first (Chrome flushes its profile/cookies) and only
//!   escalates to SIGKILL after a grace period.

use super::ChromeError;

/// Whether this host looks able to run Chrome's own sandbox.
///
/// Deliberately conservative: it only reports a blocker it can prove, because a
/// false negative here would refuse to start for a user whose sandbox is fine.
/// Anything undetectable resolves to `Available` and, if Chrome then fails to
/// come up, `ChromeError::NotReady` carries Chrome's own stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxSupport {
    /// No detectable blocker.
    Available,
    /// Chrome refuses to enable its sandbox under uid 0 and exits immediately.
    BlockedRunningAsRoot,
    /// Linux with unprivileged user namespaces disabled and no setuid helper.
    BlockedNoUserNamespaces,
}

impl SandboxSupport {
    /// Why the sandbox can't run, and what the operator can do about it.
    fn explain(self) -> Option<(&'static str, &'static str)> {
        match self {
            SandboxSupport::Available => None,
            SandboxSupport::BlockedRunningAsRoot => Some((
                "this process runs as root (uid 0), and Chrome will not enable its \
                 sandbox for a root user",
                "Run NeoBrowser as an unprivileged user. In a container, add a non-root \
                 user and `USER` it, or grant the container SYS_ADMIN and unprivileged \
                 user namespaces. Only as a last resort, set \
                 NEOBROWSER_ALLOW_NO_SANDBOX=1 to run unsandboxed.",
            )),
            SandboxSupport::BlockedNoUserNamespaces => Some((
                "this Linux host has unprivileged user namespaces disabled and no \
                 setuid chrome-sandbox helper next to the Chrome binary",
                "Enable them with `sysctl -w kernel.unprivileged_userns_clone=1` (or \
                 `user.max_user_namespaces=15000`), or install Chrome's setuid sandbox \
                 helper. Only as a last resort, set NEOBROWSER_ALLOW_NO_SANDBOX=1 to run \
                 unsandboxed.",
            )),
        }
    }
}

/// Detect whether Chrome's sandbox can run here. See [`SandboxSupport`].
pub fn sandbox_support() -> SandboxSupport {
    // Root is the real reason nearly every tool ends up shipping --no-sandbox:
    // Chrome hard-refuses the sandbox as uid 0. This check is exact, not a guess.
    #[cfg(unix)]
    // SAFETY: `geteuid` takes no arguments, cannot fail, and returns a plain integer. It
    // touches no memory we own. This is the minimum-risk shape an FFI call can have.
    if unsafe { libc::geteuid() } == 0 {
        return SandboxSupport::BlockedRunningAsRoot;
    }
    #[cfg(target_os = "linux")]
    if !linux_userns_available() && !setuid_sandbox_helper_present() {
        return SandboxSupport::BlockedNoUserNamespaces;
    }
    SandboxSupport::Available
}

/// Can an unprivileged process create a user namespace? Unreadable sysctls mean
/// "can't tell", which resolves to `true` — never refuse on missing evidence.
#[cfg(target_os = "linux")]
fn linux_userns_available() -> bool {
    fn sysctl(path: &str) -> Option<i64> {
        std::fs::read_to_string(path).ok()?.trim().parse().ok()
    }
    // Debian/Ubuntu's downstream toggle: an explicit 0 is a hard block.
    if sysctl("/proc/sys/kernel/unprivileged_userns_clone") == Some(0) {
        return false;
    }
    // Ubuntu 23.10+ (so ubuntu-24.04 runners, the default GitHub image) leaves
    // max_user_namespaces high but has AppArmor deny userns to unconfined
    // binaries. Chrome downloaded to a CI workspace is unconfined, so its sandbox
    // fails here while the two sysctls above look perfectly healthy — this is the
    // check whose absence turns into an opaque launch timeout.
    if sysctl("/proc/sys/kernel/apparmor_restrict_unprivileged_userns") == Some(1) {
        return false;
    }
    // Upstream limit: 0 means no user namespaces for anyone.
    sysctl("/proc/sys/user/max_user_namespaces").is_none_or(|max| max > 0)
}

/// Chrome's SUID sandbox helper, the fallback when user namespaces are off.
#[cfg(target_os = "linux")]
fn setuid_sandbox_helper_present() -> bool {
    use std::os::unix::fs::MetadataExt;
    // Imported inside the cfg block: this whole function is Linux-only, and a
    // top-level import would be an unused-import warning everywhere else. Its
    // absence only broke the Linux build, so a macOS-only run never saw it.
    use crate::chrome::chrome_bin;
    chrome_bin()
        .parent()
        .map(|dir| dir.join("chrome-sandbox"))
        .and_then(|helper| std::fs::metadata(helper).ok())
        // setuid bit AND owned by root, or it grants nothing.
        .is_some_and(|m| m.mode() & 0o4000 != 0 && m.uid() == 0)
}

/// How far the operator has opted out of the sandbox via
/// `NEOBROWSER_ALLOW_NO_SANDBOX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoSandboxOptIn {
    /// Default: the sandbox is required.
    No,
    /// Unsandboxed is allowed, but not together with real-profile cookies.
    Yes,
    /// Unsandboxed is allowed even while importing real sessions.
    YesWithRealProfile,
}

/// Has the operator opted out of the sandbox at all? For `doctor` and any other
/// surface that has to disclose the effective security posture.
pub fn no_sandbox_opt_in_active() -> bool {
    no_sandbox_opt_in() != NoSandboxOptIn::No
}

pub(super) fn no_sandbox_opt_in() -> NoSandboxOptIn {
    match std::env::var("NEOBROWSER_ALLOW_NO_SANDBOX") {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "with-real-profile" => NoSandboxOptIn::YesWithRealProfile,
            "1" | "true" | "yes" | "on" => NoSandboxOptIn::Yes,
            _ => NoSandboxOptIn::No,
        },
        Err(_) => NoSandboxOptIn::No,
    }
}

/// Is a real Chrome profile being used as a cookie source for this session?
fn real_profile_requested() -> Option<String> {
    std::env::var("NEOBROWSER_REAL_PROFILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// Decide whether this launch passes `--no-sandbox`, refusing the combinations
/// that would quietly trade the user's machine for convenience.
///
/// The matrix, in one place so it can be tested without spawning Chrome:
/// - sandbox available, no opt-in      -> sandboxed (the ordinary path)
/// - sandbox blocked, no opt-in        -> refuse, with the blocker and the fix
/// - opt-in, no real profile           -> unsandboxed, warn on every launch
/// - opt-in + real profile             -> refuse unless the opt-in names that case
pub(super) fn resolve_sandbox() -> Result<bool, ChromeError> {
    let opt_in = no_sandbox_opt_in();
    let support = sandbox_support();
    if opt_in == NoSandboxOptIn::No {
        return match support.explain() {
            None => Ok(false),
            Some((reason, hint)) => Err(ChromeError::SandboxUnavailable { reason, hint }),
        };
    }
    if let Some(profile) = real_profile_requested() {
        if opt_in != NoSandboxOptIn::YesWithRealProfile {
            return Err(ChromeError::NoSandboxWithRealProfile { profile });
        }
        tracing::warn!(
            profile = %profile,
            "SECURITY: Chrome is running WITHOUT its sandbox while holding real-profile \
             cookies. A single renderer exploit on any page reaches both this machine and \
             those logged-in sessions. Unset NEOBROWSER_ALLOW_NO_SANDBOX as soon as you can."
        );
        return Ok(true);
    }
    tracing::warn!(
        "SECURITY: Chrome is running WITHOUT its sandbox (NEOBROWSER_ALLOW_NO_SANDBOX). \
         A compromised page can escape the renderer and reach this machine. This is not a \
         supported configuration for untrusted browsing."
    );
    Ok(true)
}
