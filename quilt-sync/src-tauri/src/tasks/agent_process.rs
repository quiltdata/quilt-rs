//! The agent process a run spawns, recorded so that the sweep can stop one a
//! hard kill of the app left running.

use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;

/// A process as it was when spawned. Its start time tells it apart from a
/// later process given the same PID.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spawned {
    pub pid: u32,
    started: u64,
}

impl Spawned {
    /// `None` when the start time can't be read: the process is gone, or this
    /// platform isn't macOS or Linux.
    pub fn of(pid: u32) -> Option<Self> {
        started(pid).map(|started| Self { pid, started })
    }

    fn running(self) -> bool {
        started(self.pid) == Some(self.started)
    }

    /// SIGTERM, then SIGKILL if it outlives `grace`, waiting up to a second
    /// `grace` for it to go; whether it has. A process that has exited, or
    /// whose PID now names another, is left alone.
    pub async fn stop(self, grace: Duration) -> bool {
        for signalled in [Signal::Terminate, Signal::Kill] {
            if !self.running() {
                return true;
            }
            signal(self.pid, signalled);
            let deadline = tokio::time::Instant::now() + grace;
            while self.running() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        !self.running()
    }
}

#[derive(Clone, Copy)]
enum Signal {
    Terminate,
    Kill,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn signal(pid: u32, signal: Signal) {
    let signal = match signal {
        Signal::Terminate => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: `kill` takes plain integers and touches no memory of ours.
        unsafe { libc::kill(pid, signal) };
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn signal(_: u32, _: Signal) {}

/// When `pid` started, in microseconds since the epoch; `None` once it has
/// exited, a zombie included.
#[cfg(target_os = "macos")]
fn started(pid: u32) -> Option<u64> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: `proc_bsdinfo` is plain data, valid zeroed.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid out buffer of `size` bytes.
    let read =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (read == size && info.pbi_status != libc::SZOMB)
        .then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

/// When `pid` started, in clock ticks since boot; `None` once it has exited,
/// a zombie included.
#[cfg(target_os = "linux")]
fn started(pid: u32) -> Option<u64> {
    let stat = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
    // The fields after the command name, which may itself hold spaces,
    // parentheses and bytes that aren't UTF-8: the state is field 3, the
    // start time field 22.
    let after_name = stat.iter().rposition(|byte| *byte == b')')? + 1;
    let mut fields = std::str::from_utf8(&stat[after_name..])
        .ok()?
        .split_whitespace();
    if matches!(fields.next()?, "Z" | "X") {
        return None;
    }
    fields.nth(18)?.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn started(_: u32) -> Option<u64> {
    None
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    use std::os::unix::process::ExitStatusExt;
    use std::process::Child;
    use std::process::Command;

    fn spawn(script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .spawn()
            .expect("sh starts")
    }

    /// The signal `child` exited on, waiting up to two seconds for it; `None`
    /// while it still runs.
    async fn exit_signal(child: &mut Child) -> Option<i32> {
        for _ in 0..40 {
            if let Some(status) = child.try_wait().expect("waited") {
                return status.signal();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        child.kill().expect("killed");
        child.wait().expect("waited");
        None
    }

    #[tokio::test]
    async fn a_running_agent_is_stopped_with_sigterm() {
        let mut child = spawn("exec sleep 60");
        let agent = Spawned::of(child.id()).expect("its start time");

        assert!(agent.stop(Duration::from_secs(5)).await, "reported gone");
        assert_eq!(exit_signal(&mut child).await, Some(libc::SIGTERM));
    }

    #[tokio::test]
    async fn an_agent_that_ignores_sigterm_is_killed_after_the_grace() {
        let mut child = Command::new("sh")
            .args(["-c", "trap '' TERM; echo ready; exec sleep 60"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("sh starts");
        let mut ready = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().expect("stdout")),
            &mut ready,
        )
        .expect("the trap is set");
        let agent = Spawned::of(child.id()).expect("its start time");

        assert!(
            agent.stop(Duration::from_millis(200)).await,
            "reported gone"
        );
        assert_eq!(exit_signal(&mut child).await, Some(libc::SIGKILL));
    }

    /// The PID was given to another process after the agent's exited.
    #[tokio::test]
    async fn a_process_that_started_at_another_time_is_left_alone() {
        let mut child = spawn("exec sleep 60");
        let agent = Spawned::of(child.id()).expect("its start time");
        let other = Spawned {
            started: agent.started + 1,
            ..agent
        };

        other.stop(Duration::from_millis(200)).await;

        assert!(child.try_wait().expect("waited").is_none(), "still running");
        child.kill().expect("killed");
        child.wait().expect("waited");
    }
}
