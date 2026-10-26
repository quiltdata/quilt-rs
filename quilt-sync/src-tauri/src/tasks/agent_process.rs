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

    /// SIGTERM to it and the process group it leads, then SIGKILL if any of
    /// them outlives `grace`, waiting up to a second `grace` for them to go;
    /// whether they have. Left alone unless it still runs: once it has exited,
    /// its PID may name another process.
    pub async fn stop(self, grace: Duration) -> bool {
        if !self.running() {
            return true;
        }
        for signalled in [Signal::Terminate, Signal::Kill] {
            if !self.alive() {
                return true;
            }
            if self.running() {
                signal(self.pid, signalled);
            }
            signal_group(self.pid, signalled);
            let deadline = tokio::time::Instant::now() + grace;
            while self.alive() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        !self.alive()
    }

    /// It or any of its group: an agent recorded by an earlier build leads no
    /// group of its own.
    fn alive(self) -> bool {
        self.running() || group_running(self.pid)
    }
}

/// The process group its PID leads, sent SIGKILL when dropped.
pub struct Group(pub u32);

impl Drop for Group {
    fn drop(&mut self) {
        signal_group(self.0, Signal::Kill);
    }
}

#[derive(Clone, Copy)]
enum Signal {
    Terminate,
    Kill,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn signal_number(signal: Signal) -> libc::c_int {
    match signal {
        Signal::Terminate => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn signal(pid: u32, signal: Signal) {
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: `kill` takes plain integers and touches no memory of ours.
        unsafe { libc::kill(pid, signal_number(signal)) };
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn signal_group(leader: u32, signal: Signal) {
    if let Ok(group) = libc::pid_t::try_from(leader) {
        // SAFETY: `killpg` takes plain integers and touches no memory of ours.
        unsafe { libc::killpg(group, signal_number(signal)) };
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn signal(_: u32, _: Signal) {}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn signal_group(_: u32, _: Signal) {}

/// Whether any of the group `leader` leads has yet to exit.
fn group_running(leader: u32) -> bool {
    members(leader)
        .into_iter()
        .any(|pid| started(pid).is_some())
}

#[cfg(target_os = "macos")]
fn members(leader: u32) -> Vec<u32> {
    let Ok(group) = libc::pid_t::try_from(leader) else {
        return Vec::new();
    };
    // SAFETY: a null buffer asks only for the count.
    let count = unsafe { libc::proc_listpgrppids(group, std::ptr::null_mut(), 0) };
    // Room for processes that join between the two calls.
    let mut pids: Vec<libc::pid_t> = vec![0; usize::try_from(count).unwrap_or(0) + 16];
    let Ok(size) = libc::c_int::try_from(std::mem::size_of_val(pids.as_slice())) else {
        return Vec::new();
    };
    // SAFETY: `pids` is a valid out buffer of `size` bytes.
    let count = unsafe { libc::proc_listpgrppids(group, pids.as_mut_ptr().cast(), size) };
    pids.truncate(usize::try_from(count).unwrap_or(0));
    pids.into_iter()
        .filter_map(|pid| u32::try_from(pid).ok())
        .collect()
}

#[cfg(target_os = "linux")]
fn members(leader: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter(|pid| {
            // The group is field 5.
            stat_from_state(*pid).and_then(|stat| stat.split_whitespace().nth(2)?.parse().ok())
                == Some(leader)
        })
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn members(_: u32) -> Vec<u32> {
    Vec::new()
}

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
/// a zombie with no thread left included.
#[cfg(target_os = "linux")]
fn started(pid: u32) -> Option<u64> {
    let stat = stat_from_state(pid)?;
    let mut fields = stat.split_whitespace();
    // A zombie leader may still have threads at work.
    let threads = || std::fs::read_dir(format!("/proc/{pid}/task")).map_or(0, Iterator::count);
    // The state is field 3, the start time field 22.
    match fields.next()? {
        "X" => return None,
        "Z" if threads() <= 1 => return None,
        _ => {}
    }
    fields.nth(18)?.parse().ok()
}

/// `/proc/<pid>/stat` from field 3 on, after the command name, which may
/// itself hold spaces, parentheses and bytes that aren't UTF-8.
#[cfg(target_os = "linux")]
fn stat_from_state(pid: u32) -> Option<String> {
    let stat = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
    let after_name = stat.iter().rposition(|byte| *byte == b')')? + 1;
    String::from_utf8(stat[after_name..].to_vec()).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn started(_: u32) -> Option<u64> {
    None
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    use std::os::unix::process::CommandExt;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Child;
    use std::process::Command;

    /// `script` leading a process group of its own, as a run spawns its agent,
    /// and the first line it prints.
    fn spawn(script: &str) -> (Child, String) {
        let mut child = Command::new("sh")
            .args(["-c", script])
            .process_group(0)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("sh starts");
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().expect("stdout")),
            &mut line,
        )
        .expect("it printed");
        (child, line.trim().to_string())
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
        let (mut child, _) = spawn("echo ready; exec sleep 60");
        let agent = Spawned::of(child.id()).expect("its start time");

        assert!(agent.stop(Duration::from_secs(5)).await, "reported gone");
        assert_eq!(exit_signal(&mut child).await, Some(libc::SIGTERM));
    }

    #[tokio::test]
    async fn an_agent_that_ignores_sigterm_is_killed_after_the_grace() {
        let (mut child, _) = spawn("trap '' TERM; echo ready; exec sleep 60");
        let agent = Spawned::of(child.id()).expect("its start time");

        assert!(
            agent.stop(Duration::from_millis(200)).await,
            "reported gone"
        );
        assert_eq!(exit_signal(&mut child).await, Some(libc::SIGKILL));
    }

    /// The agent goes on SIGTERM; a tool it started ignores it.
    #[tokio::test]
    async fn the_tools_an_agent_started_are_stopped_with_it() {
        let (mut child, tool) = spawn("sh -c 'trap \"\" TERM; echo $$; exec sleep 60' & wait");
        let tool: u32 = tool.parse().expect("the tool's PID");
        let agent = Spawned::of(child.id()).expect("its start time");

        assert!(
            agent.stop(Duration::from_millis(200)).await,
            "reported gone"
        );
        assert_eq!(exit_signal(&mut child).await, Some(libc::SIGTERM));
        assert_eq!(started(tool), None, "the tool is gone too");
    }

    /// The PID was given to another process after the agent's exited.
    #[tokio::test]
    async fn a_process_that_started_at_another_time_is_left_alone() {
        let (mut child, _) = spawn("echo ready; exec sleep 60");
        let agent = Spawned::of(child.id()).expect("its start time");
        let other = Spawned {
            started: agent.started + 1,
            ..agent
        };

        other.stop(Duration::from_millis(200)).await;

        assert_eq!(exit_signal(&mut child).await, None, "still running");
    }
}
