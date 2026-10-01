// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::io;
use tokio::process::{Child, Command};

/// Own descendants until completion or cancellation.
pub(super) fn spawn_owned(command: &mut Command) -> io::Result<(Child, ProcessTree)> {
    command.kill_on_drop(true);
    #[cfg(unix)]
    {
        command.process_group(0);
        let child = command.spawn()?;
        let tree = ProcessTree(Some(
            child.id().expect("newly spawned child has an ID") as i32
        ));
        Ok((child, tree))
    }
    #[cfg(windows)]
    {
        windows::spawn(command)
    }
}

pub(super) async fn output(command: &mut Command) -> io::Result<std::process::Output> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let (child, _tree) = spawn_owned(command)?;
    child.wait_with_output().await
}

#[cfg(unix)]
pub(super) struct ProcessTree(Option<i32>);

#[cfg(unix)]
impl ProcessTree {
    pub(super) fn detach(mut self) -> io::Result<()> {
        self.0 = None;
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        let Some(group) = self.0 else {
            return;
        };
        // SAFETY: a negative PID addresses only the group created for this command.
        if unsafe { libc::kill(-group, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                tracing::warn!("Failed to terminate process group {group}: {error}");
            }
        }
    }
}

#[cfg(windows)]
pub(super) struct ProcessTree {
    _job: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl ProcessTree {
    pub(super) fn detach(self) -> io::Result<()> {
        windows::set_kill_on_close(&self._job, false)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };

    fn own_handle(handle: HANDLE) -> io::Result<OwnedHandle> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: these APIs return a new handle owned by the caller.
            Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
        }
    }

    pub(super) fn set_kill_on_close(job: &OwnedHandle, kill: bool) -> io::Result<()> {
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation:
                windows_sys::Win32::System::JobObjects::JOBOBJECT_BASIC_LIMIT_INFORMATION {
                    LimitFlags: if kill {
                        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                    } else {
                        0
                    },
                    ..Default::default()
                },
            ..Default::default()
        };
        // SAFETY: the buffer has the required size and lives through the call.
        if unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, ProcessTree)> {
        // SAFETY: null security/name pointers request a private, unnamed job.
        let job = own_handle(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
        set_kill_on_close(&job, true)?;
        // Suspend before job assignment so the shell cannot spawn unowned children.
        command.creation_flags(CREATE_SUSPENDED);
        let child = command.spawn()?;
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("Process handle is unavailable"))?;
        // SAFETY: both handles are live and the child has not started running.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let tree = ProcessTree { _job: job };
        resume_main_thread(child.id().expect("newly spawned child has an ID"))?;
        Ok((child, tree))
    }

    // Tokio retains the process handle but not the primary thread handle.
    fn resume_main_thread(pid: u32) -> io::Result<()> {
        // SAFETY: the snapshot and entry buffer are owned here.
        let snapshot = own_handle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == pid {
                // A suspended, newly-created process has only its primary thread.
                let thread = own_handle(unsafe {
                    OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID)
                })?;
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        Err(io::Error::other(format!(
            "Cannot find the primary thread of process {pid}"
        )))
    }
}
