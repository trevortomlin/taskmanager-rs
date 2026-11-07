//! Ending processes safely.
//!
//! A PID alone is not a stable identity: if the process the user picked exits,
//! the kernel may hand its PID to an unrelated new process. We therefore open a
//! pidfd (a handle that always refers to one specific process), confirm through
//! it that the process still has the start time we displayed, and only then
//! signal it via the pidfd.

use rustix::{
    io::Errno,
    process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal},
};

use crate::sampler::process_start_time;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalKind {
    /// SIGTERM: ask the process to exit cleanly.
    Terminate,
    /// SIGKILL: stop the process immediately.
    Kill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalError {
    /// The process exited (its PID may already belong to another process).
    Exited,
    /// The process belongs to another user.
    PermissionDenied,
    Other(Errno),
}

impl From<Errno> for SignalError {
    fn from(err: Errno) -> Self {
        match err {
            Errno::SRCH => SignalError::Exited,
            Errno::PERM => SignalError::PermissionDenied,
            other => SignalError::Other(other),
        }
    }
}

/// Signals the process `pid` only if it is still the instance that started at `start_time`.
pub fn signal_process(pid: u32, start_time: u64, kind: SignalKind) -> Result<(), SignalError> {
    let raw = i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or(SignalError::Exited)?;
    let pidfd = pidfd_open(raw, PidfdFlags::empty())?;
    // The pidfd is now pinned to one process, so this check can't race with PID reuse.
    if process_start_time(pid) != Some(start_time) {
        return Err(SignalError::Exited);
    }
    let signal = match kind {
        SignalKind::Terminate => Signal::TERM,
        SignalKind::Kill => Signal::KILL,
    };
    pidfd_send_signal(&pidfd, signal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Child, Command},
    };

    use super::*;

    fn spawn_sleeper() -> (Child, u64) {
        let child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let start = process_start_time(child.id()).expect("child start time");
        (child, start)
    }

    #[test]
    fn terminates_the_expected_process() {
        let (mut child, start) = spawn_sleeper();
        assert_eq!(
            signal_process(child.id(), start, SignalKind::Terminate),
            Ok(())
        );
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(Signal::TERM.as_raw()));
    }

    #[test]
    fn refuses_when_start_time_does_not_match() {
        let (mut child, start) = spawn_sleeper();
        let result = signal_process(child.id(), start + 1, SignalKind::Kill);
        assert_eq!(result, Err(SignalError::Exited));
        assert!(child.try_wait().unwrap().is_none(), "process must survive");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn reports_exited_processes() {
        let (mut child, start) = spawn_sleeper();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(
            signal_process(pid, start, SignalKind::Terminate),
            Err(SignalError::Exited)
        );
    }
}
