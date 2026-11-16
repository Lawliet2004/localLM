//! Linux crash ownership for a managed process group.
//!
//! A separate, minimal process anchors the group before any workload starts.
//! Its stdin is a close-on-exec pipe owned only by LocalLM. Owner death closes
//! that pipe and the guardian kills the entire group, including itself. The
//! group id cannot be recycled while its guardian is alive. This also covers
//! grandchildren; PR_SET_PDEATHSIG alone covers only the direct child.

use std::io;
use std::process::Stdio;

pub(crate) struct Guardian {
    child: tokio::process::Child,
    _owner: tokio::process::ChildStdin,
    group: rustix::process::Pid,
}

impl Guardian {
    pub(crate) fn start() -> io::Result<Self> {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' INT TERM HUP; IFS= read -r locallm_owner; kill -9 -$$",
            ])
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let raw = child
            .id()
            .ok_or_else(|| io::Error::other("Process guardian exited during startup."))?;
        let group = rustix::process::Pid::from_raw(raw as i32)
            .ok_or_else(|| io::Error::other("Process guardian has an invalid group id."))?;
        let owner = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Process guardian has no ownership pipe."))?;
        Ok(Self {
            child,
            _owner: owner,
            group,
        })
    }

    pub(crate) fn group(&self) -> i32 {
        self.group.as_raw_nonzero().get()
    }

    pub(crate) fn kill(&mut self) -> io::Result<()> {
        self.signal(rustix::process::Signal::KILL)
    }

    pub(crate) fn signal(&self, signal: rustix::process::Signal) -> io::Result<()> {
        // The unreaped guardian retains this group's identity even if the
        // workload leader already exited. No unrelated pid is targeted.
        match rustix::process::kill_process_group(self.group, signal) {
            Ok(()) => Ok(()),
            Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for Guardian {
    fn drop(&mut self) {
        let _ = self.kill();
        // Tokio reaps the guardian after Drop; no synchronous wait blocks a
        // runtime worker. On process exit the pipe supplies crash cleanup.
        let _ = self.child.start_kill();
    }
}
