use std::fmt;
use std::fs;
use std::io;

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
    let mut procs = Vec::new();

    let Ok(entries) = fs::read_dir("/proc") else {
        return procs;
    };

    for entry in entries.flatten() {
        let pid: u32 = match entry.file_name().to_string_lossy().parse() {
            Ok(pid) => pid,
            Err(_) => continue,
        };

        // process can die between the readdir and this read
        let Ok(comm) = fs::read_to_string(format!("/proc/{pid}/comm")) else {
            continue;
        };

        procs.push(Proc {
            pid,
            name: comm.trim().to_string(),
        });
    }

    procs.sort_by_key(|p| p.pid);
    procs
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
