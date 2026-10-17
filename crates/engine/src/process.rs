use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::io::Read;

use xedit_core::{Address, Region};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub cmdline: Option<String>,
}

impl Proc {
    /// Matches the short name, the full command line, or an exact pid.
    pub fn matches(&self, needle: &str) -> bool {
        if let Ok(pid) = needle.parse::<u32>() {
            if self.pid == pid {
                return true;
            }
        }

        let needle = needle.to_lowercase();
        self.name.to_lowercase().contains(&needle)
            || self
                .cmdline
                .as_deref()
                .is_some_and(|c| c.to_lowercase().contains(&needle))
    }

    /// The command line, or a bracketed name for kernel threads, which have none.
    pub fn command(&self) -> String {
        match self.cmdline.as_deref() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => format!("[{}]", self.name),
        }
    }
}

impl fmt::Display for Proc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:>7}  {}", self.pid, self.command())
    }
}

/// Every numeric directory under /proc is a running process.
pub fn list() -> Vec<Proc> {
    let mut pids: Vec<u32> = Vec::with_capacity(512);

    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        pids.push(pid);
    }

    if pids.is_empty() {
        return Vec::new();
    }

    pids.sort_unstable();

    // reading 400+ tiny files is all syscalls, so split it over a few cores.
    // measured: 3 is the knee, more threads just add spawn and join cost
    let lanes = std::thread::available_parallelism()
        .map_or(3, |n| n.get())
        .min(3);
    let chunk = pids.len().div_ceil(lanes);

    let mut procs = Vec::with_capacity(pids.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = pids
            .chunks(chunk)
            .map(|slice| scope.spawn(move || read_procs(slice)))
            .collect();

        for h in handles {
            procs.extend(h.join().unwrap_or_default());
        }
    });

    procs.sort_unstable_by_key(|p| p.pid);
    procs
}

fn read_procs(pids: &[u32]) -> Vec<Proc> {
    // reused per lane, otherwise we allocate a path and a string per process
    let mut path = String::with_capacity(24);
    let mut comm = [0u8; 64];
    let mut argv = Vec::with_capacity(512);
    let mut out = Vec::with_capacity(pids.len());

    for &pid in pids {
        path.clear();
        let _ = write!(path, "/proc/{pid}/comm");

        // process can die between the readdir and this open
        let Ok(mut file) = fs::File::open(&path) else {
            continue;
        };

        // comm is at most 16 bytes, one read is enough and skips the EOF read
        let Ok(n) = file.read(&mut comm) else {
            continue;
        };

        let end = comm[..n].iter().position(|&b| b == b'\n').unwrap_or(n);
        let Ok(name) = std::str::from_utf8(&comm[..end]) else {
            continue;
        };

        path.clear();
        let _ = write!(path, "/proc/{pid}/cmdline");
        let cmdline = read_cmdline(&path, &mut argv);

        out.push(Proc {
            pid,
            name: name.to_string(),
            cmdline,
        });
    }

    out
}

/// argv, which the kernel hands over NUL separated. Empty for kernel threads.
/// Reads into a reused buffer, so the usual case is one read with no allocation.
fn read_cmdline(path: &str, buf: &mut Vec<u8>) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    buf.clear();

    loop {
        let start = buf.len();
        buf.resize(start + 512, 0);
        match file.read(&mut buf[start..]) {
            Ok(0) => {
                buf.truncate(start);
                break;
            }
            Ok(n) => {
                buf.truncate(start + n);
                if n < 512 {
                    break;
                }
            }
            Err(_) => return None,
        }
    }

    Some(join_cmdline(buf.as_slice()))
}

fn join_cmdline(bytes: &[u8]) -> String {
    let mut line = String::with_capacity(bytes.len());

    for arg in bytes.split(|&b| b == 0).filter(|a| !a.is_empty()) {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&String::from_utf8_lossy(arg));
    }

    line
}

pub fn regions(pid: u32) -> io::Result<Vec<Region>> {
    let text = fs::read_to_string(format!("/proc/{pid}/maps"))?;
    Ok(parse_maps(&text))
}

fn parse_maps(text: &str) -> Vec<Region> {
    let mut out = Vec::new();

    for line in text.lines() {
        // start-end perms offset dev inode [pathname]
        let mut fields = line.splitn(6, ' ');
        let (Some(range), Some(perms)) = (fields.next(), fields.next()) else {
            continue;
        };

        let Some((start, end)) = range.split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) = (u64::from_str_radix(start, 16), u64::from_str_radix(end, 16))
        else {
            continue;
        };

        // "rw-p", always at least three chars
        let p = perms.as_bytes();
        if p.len() < 3 {
            continue;
        }

        // skip offset, dev, inode -> pathname (may contain spaces)
        let path = fields
            .nth(3)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        out.push(Region {
            start: Address(start),
            end: Address(end),
            read: p[0] == b'r',
            write: p[1] == b'w',
            exec: p[2] == b'x',
            path,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_map_line() {
        let text = "5c0012a000-5c0012b000 rw-p 00000000 00:00 0 \n\
                    7f2a10000000-7f2a10021000 r-xp 00000000 08:01 12345 /usr/bin/cat";
        let rs = parse_maps(text);

        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].start, Address(0x5c0012a000));
        assert!(rs[0].write);
        assert!(!rs[0].exec);
        assert_eq!(rs[0].path, None);

        assert_eq!(rs[1].path.as_deref(), Some("/usr/bin/cat"));
        assert!(rs[1].read && rs[1].exec && !rs[1].write);
    }

    #[test]
    fn path_with_spaces_survives() {
        let text = "1000-2000 rw-p 00000000 00:00 0 /home/me/my file.so";
        let rs = parse_maps(text);
        assert_eq!(rs[0].path.as_deref(), Some("/home/me/my file.so"));
    }

    #[test]
    fn list_finds_this_process() {
        let me = std::process::id();
        assert!(list().iter().any(|p| p.pid == me));
    }

    #[test]
    fn cmdline_is_joined_with_spaces() {
        assert_eq!(
            join_cmdline(b"python3\0test/coins.py\0"),
            "python3 test/coins.py"
        );
        assert_eq!(join_cmdline(b"/usr/bin/sleep\0\0"), "/usr/bin/sleep");
        assert_eq!(join_cmdline(b""), "");
    }

    #[test]
    fn matches_name_cmdline_and_pid() {
        let p = Proc {
            pid: 42,
            name: "python3".to_string(),
            cmdline: Some("python3 test/coins.py".to_string()),
        };

        assert!(p.matches("coins"));
        assert!(p.matches("Python"));
        assert!(p.matches("42"));
        assert!(!p.matches("firefox"));
    }
}
