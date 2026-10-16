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
}

impl fmt::Display for Proc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:>7}  {}", self.pid, self.name)
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
            .map(|slice| scope.spawn(move || read_names(slice)))
            .collect();

        for h in handles {
            procs.extend(h.join().unwrap_or_default());
        }
    });

    procs.sort_unstable_by_key(|p| p.pid);
    procs
}

fn read_names(pids: &[u32]) -> Vec<Proc> {
    // reused per lane, otherwise we allocate a path and a string per process
    let mut path = String::with_capacity(24);
    let mut buf = [0u8; 64];
    let mut out = Vec::with_capacity(pids.len());

    for &pid in pids {
        path.clear();
        let _ = write!(path, "/proc/{pid}/comm");

        // process can die between the readdir and this open
        let Ok(mut file) = fs::File::open(&path) else {
            continue;
        };

        // comm is at most 16 bytes, one read is enough and skips the EOF read
        let Ok(n) = file.read(&mut buf) else {
            continue;
        };

        let end = buf[..n].iter().position(|&b| b == b'\n').unwrap_or(n);
        let Ok(name) = std::str::from_utf8(&buf[..end]) else {
            continue;
        };

        out.push(Proc {
            pid,
            name: name.to_string(),
        });
    }

    out
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
}
