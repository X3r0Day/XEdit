use std::fs;
use std::io;

use xedit_core::{Address, Error, Region};

use crate::backend::MemoryBackend;
use crate::process;

/// A process we hold a handle to, read and written by pid.
pub struct RemoteProcess {
    pid: u32,
}

impl RemoteProcess {
    pub fn open(pid: u32) -> Result<Self, Error> {
        if fs::metadata(format!("/proc/{pid}")).is_err() {
            return Err(Error::NoSuchProcess(pid));
        }
        Ok(RemoteProcess { pid })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn translate(&self, e: io::Error, addr: Address) -> Error {
        match e.raw_os_error() {
            Some(libc::EACCES) | Some(libc::EPERM) => Error::Denied(hint(self.pid)),
            Some(libc::EFAULT) => Error::Unmapped(addr.0),
            Some(libc::ESRCH) => Error::NoSuchProcess(self.pid),
            _ => Error::Io(e),
        }
    }
}

impl MemoryBackend for RemoteProcess {
    fn regions(&self) -> Result<Vec<Region>, Error> {
        Ok(process::regions(self.pid)?)
    }

    fn read(&self, addr: Address, buf: &mut [u8]) -> Result<usize, Error> {
        let local = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        let remote = libc::iovec {
            iov_base: addr.0 as *mut libc::c_void,
            iov_len: buf.len(),
        };

        // one syscall, no poking one byte at a time
        let n =
            unsafe { libc::process_vm_readv(self.pid as libc::pid_t, &local, 1, &remote, 1, 0) };

        if n < 0 {
            return Err(self.translate(io::Error::last_os_error(), addr));
        }
        Ok(n as usize)
    }

    fn write(&self, addr: Address, bytes: &[u8]) -> Result<usize, Error> {
        let local = libc::iovec {
            iov_base: bytes.as_ptr() as *mut libc::c_void,
            iov_len: bytes.len(),
        };
        let remote = libc::iovec {
            iov_base: addr.0 as *mut libc::c_void,
            iov_len: bytes.len(),
        };

        let n =
            unsafe { libc::process_vm_writev(self.pid as libc::pid_t, &local, 1, &remote, 1, 0) };

        if n < 0 {
            return Err(self.translate(io::Error::last_os_error(), addr));
        }
        Ok(n as usize)
    }
}

/// 0 = anyone can trace, 1 = only children, and so on. Only exists when yama is on.
pub fn ptrace_scope() -> Option<u8> {
    fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn hint(pid: u32) -> String {
    match ptrace_scope() {
        Some(0) => format!("can't reach pid {pid}: try running xedit with CAP_SYS_PTRACE"),
        Some(n) => format!(
            "can't reach pid {pid}: yama ptrace_scope is {n}, so only children can be traced. \
             run `sudo sysctl kernel.yama.ptrace_scope=0` or give xedit CAP_SYS_PTRACE"
        ),
        None => format!("can't reach pid {pid}: not permitted"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn reads_a_child_process() {
        let mut child = Command::new("sleep").arg("2").spawn().unwrap();
        let pid = child.id();
        wait_until_running(pid);

        let proc = RemoteProcess::open(pid).unwrap();
        let regions = proc.regions().unwrap();
        let file_backed = regions
            .iter()
            .find(|r| r.read && r.path.is_some())
            .expect("no readable region");

        let mut buf = [0u8; 16];
        match proc.read(file_backed.start, &mut buf) {
            Ok(n) => assert_eq!(n, 16),
            // some sandboxes block ptrace outright, that's not our bug
            Err(Error::Denied(_)) => {}
            Err(e) => panic!("unexpected: {e}"),
        }

        child.kill().ok();
        child.wait().ok();
    }

    // spawn hands back before the child has replaced its image, and reading a
    // map point out of the parent's address space proves nothing
    fn wait_until_running(pid: u32) {
        let stat = format!("/proc/{pid}/stat");
        for _ in 0..200 {
            if let Ok(s) = std::fs::read_to_string(&stat) {
                if let Some((_, rest)) = s.rsplit_once(')') {
                    if rest.trim_start().starts_with('S') {
                        return;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn missing_pid_is_reported() {
        assert!(matches!(
            RemoteProcess::open(0x7fff_fff0),
            Err(Error::NoSuchProcess(_))
        ));
    }
}
