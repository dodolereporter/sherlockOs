//! The processes sherlockd starts never outlive it: Claude Code, the proxy that Claude Code
//! starts, and the MCP servers. Without this, a sherlockd killed (a crash, SIGKILL) would leave
//! them to finish their turn in the background, outside any timeout.

use std::io;

use tokio::process::{Child, Command};

/// The kernel kills the process started by `command` with SIGKILL when the thread that started
/// it ends, and so when sherlockd ends, however it ends.
pub fn die_with_parent(command: &mut Command) {
    let parent = libc::pid_t::try_from(std::process::id()).unwrap_or(0);
    // SAFETY: the closure runs in the child between fork and exec. It only calls prctl(2) and
    // getppid(2), which are async-signal-safe, and builds its error from a raw errno without
    // allocating.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(io::Error::last_os_error());
            }
            // The parent may have died between fork and prctl: the signal would never come.
            if libc::getppid() != parent {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

/// The same for this process: the proxy, started by Claude Code, dies with it.
pub fn die_with_own_parent() -> io::Result<()> {
    // SAFETY: prctl(2) with PR_SET_PDEATHSIG takes a signal number and touches no memory.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `command` in a process group of its own, which dies with sherlockd: what it starts (the
/// proxy, the MCP servers) is in that group too, and `kill_group` stops them all at once.
pub fn own_group(command: &mut Command) {
    command.process_group(0);
    die_with_parent(command);
}

/// Kills with SIGKILL the process group that `child` leads (`own_group`), if it still runs.
pub fn kill_group(child: &Child) {
    let Some(pid) = child.id().and_then(|id| libc::pid_t::try_from(id).ok()) else {
        return;
    };
    // SAFETY: kill(2) takes plain integers. The group is the child's own, which has not been
    // reaped (it has an id), so the number names no other group.
    if unsafe { libc::kill(-pid, libc::SIGKILL) } == -1 {
        tracing::debug!(error = %io::Error::last_os_error(), "process group already gone");
    }
}
