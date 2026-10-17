use std::cmp::Ordering;

use xedit_core::{Address, Error, Region};

use crate::backend::MemoryBackend;

/// How much of the target we pull in per read.
const CHUNK: usize = 4 * 1024 * 1024;

/// The value kinds we can look for. Byte and string scans come later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl ValueType {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "i8" => ValueType::I8,
            "u8" => ValueType::U8,
            "i16" => ValueType::I16,
            "u16" => ValueType::U16,
            "i32" => ValueType::I32,
            "u32" => ValueType::U32,
            "i64" => ValueType::I64,
            "u64" => ValueType::U64,
            "f32" => ValueType::F32,
            "f64" => ValueType::F64,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            ValueType::I8 => "i8",
            ValueType::U8 => "u8",
            ValueType::I16 => "i16",
            ValueType::U16 => "u16",
            ValueType::I32 => "i32",
            ValueType::U32 => "u32",
            ValueType::I64 => "i64",
            ValueType::U64 => "u64",
            ValueType::F32 => "f32",
            ValueType::F64 => "f64",
        }
    }

    pub fn size(self) -> usize {
        match self {
            ValueType::I8 | ValueType::U8 => 1,
            ValueType::I16 | ValueType::U16 => 2,
            ValueType::I32 | ValueType::U32 | ValueType::F32 => 4,
            ValueType::I64 | ValueType::U64 | ValueType::F64 => 8,
        }
    }

    /// Turn the user's text into value bits for this type.
    pub fn parse_value(self, text: &str) -> Option<u64> {
        Some(match self {
            ValueType::I8 => text.parse::<i8>().ok()? as u8 as u64,
            ValueType::U8 => text.parse::<u8>().ok()? as u64,
            ValueType::I16 => text.parse::<i16>().ok()? as u16 as u64,
            ValueType::U16 => text.parse::<u16>().ok()? as u64,
            ValueType::I32 => text.parse::<i32>().ok()? as u32 as u64,
            ValueType::U32 => text.parse::<u32>().ok()? as u64,
            ValueType::I64 => text.parse::<i64>().ok()? as u64,
            ValueType::U64 => text.parse::<u64>().ok()?,
            ValueType::F32 => text.parse::<f32>().ok()?.to_bits() as u64,
            ValueType::F64 => text.parse::<f64>().ok()?.to_bits(),
        })
    }

    /// Value bits as little endian bytes, as many as the type is wide.
    pub fn encode(self, bits: u64) -> Vec<u8> {
        bits.to_le_bytes()[..self.size()].to_vec()
    }

    pub fn decode(self, bytes: &[u8]) -> u64 {
        let mut buf = [0u8; 8];
        buf[..self.size()].copy_from_slice(&bytes[..self.size()]);
        u64::from_le_bytes(buf)
    }

    /// Order two values the way this type means them, signedness and all.
    pub fn cmp(self, a: u64, b: u64) -> Ordering {
        match self {
            ValueType::I8 => (a as i8).cmp(&(b as i8)),
            ValueType::U8 => (a as u8).cmp(&(b as u8)),
            ValueType::I16 => (a as i16).cmp(&(b as i16)),
            ValueType::U16 => (a as u16).cmp(&(b as u16)),
            ValueType::I32 => (a as i32).cmp(&(b as i32)),
            ValueType::U32 => (a as u32).cmp(&(b as u32)),
            ValueType::I64 => (a as i64).cmp(&(b as i64)),
            ValueType::U64 => a.cmp(&b),
            ValueType::F32 => f32::from_bits(a as u32)
                .partial_cmp(&f32::from_bits(b as u32))
                .unwrap_or(Ordering::Equal),
            ValueType::F64 => f64::from_bits(a)
                .partial_cmp(&f64::from_bits(b))
                .unwrap_or(Ordering::Equal),
        }
    }

    pub fn format(self, bits: u64) -> String {
        match self {
            ValueType::I8 => (bits as i8).to_string(),
            ValueType::U8 => (bits as u8).to_string(),
            ValueType::I16 => (bits as i16).to_string(),
            ValueType::U16 => (bits as u16).to_string(),
            ValueType::I32 => (bits as i32).to_string(),
            ValueType::U32 => (bits as u32).to_string(),
            ValueType::I64 => (bits as i64).to_string(),
            ValueType::U64 => bits.to_string(),
            ValueType::F32 => f32::from_bits(bits as u32).to_string(),
            ValueType::F64 => f64::from_bits(bits).to_string(),
        }
    }
}

/// One address that held the value, and what it held last time we looked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub addr: Address,
    pub prev: u64,
}

/// What to keep when narrowing a previous set of hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Exact(u64),
    Changed,
    Unchanged,
    Increased,
    Decreased,
}

impl Filter {
    pub fn parse(kind: ValueType, name: &str, value: Option<&str>) -> Option<Self> {
        Some(match name {
            "changed" => Filter::Changed,
            "unchanged" => Filter::Unchanged,
            "increased" => Filter::Increased,
            "decreased" => Filter::Decreased,
            "exact" => Filter::Exact(kind.parse_value(value?)?),
            _ => return None,
        })
    }

    fn keeps(self, kind: ValueType, prev: u64, now: u64) -> bool {
        match self {
            Filter::Exact(want) => now == want,
            Filter::Changed => now != prev,
            Filter::Unchanged => now == prev,
            Filter::Increased => kind.cmp(now, prev) == Ordering::Greater,
            Filter::Decreased => kind.cmp(now, prev) == Ordering::Less,
        }
    }
}

/// Find the exact value everywhere in writable memory.
pub fn first<B: MemoryBackend + ?Sized>(
    backend: &B,
    regions: &[Region],
    kind: ValueType,
    bits: u64,
    max: Option<usize>,
) -> Result<Vec<Hit>, Error> {
    let needle = kind.encode(bits);
    let size = needle.len();
    let mut hits = Vec::new();
    let mut buf = vec![0u8; CHUNK];

    for region in regions {
        // values worth editing live in writable memory, so skip code and rodata
        if !(region.read && region.write) {
            continue;
        }

        let mut addr = region.start.0;
        while addr < region.end.0 {
            let remaining = (region.end.0 - addr) as usize;
            if remaining < size {
                break;
            }

            let want = remaining.min(CHUNK);
            let n = match backend.read(Address(addr), &mut buf[..want]) {
                Ok(n) => n,
                // a permission wall covers the whole process, so say so
                Err(e @ Error::Denied(_)) => return Err(e),
                // the mapping went away under us, step over this chunk
                Err(_) => {
                    addr += want as u64;
                    continue;
                }
            };

            if n < size {
                break;
            }

            // step one byte at a time so values at any offset are caught,
            // find_iter would skip overlapping ones
            let mut from = 0;
            while from + size <= n {
                let Some(off) = memchr::memmem::find(&buf[from..n], &needle) else {
                    break;
                };
                let at = from + off;
                hits.push(Hit {
                    addr: Address(addr + at as u64),
                    prev: bits,
                });
                if max.is_some_and(|m| hits.len() >= m) {
                    return Ok(hits);
                }
                from = at + 1;
            }

            if want < CHUNK {
                break;
            }

            // a value can sit across the seam, so step back by the needle less
            // one byte and let the next read catch it
            addr += (n - (size - 1)) as u64;
        }
    }

    Ok(hits)
}

/// Walk the previous hits, reading each one's current value in bulk and
/// keeping the ones `keep(prev, now)` agrees with.
fn reread<B: MemoryBackend + ?Sized>(
    backend: &B,
    regions: &[Region],
    kind: ValueType,
    hits: &[Hit],
    keep: &dyn Fn(u64, u64) -> bool,
) -> Result<Vec<Hit>, Error> {
    let size = kind.size();
    let mut out = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    let mut i = 0;

    for region in regions {
        if !region.read {
            continue;
        }

        // hits arrive sorted, so walk them alongside the regions
        while i < hits.len() && hits[i].addr.0 + size as u64 <= region.start.0 {
            i += 1;
        }
        if i >= hits.len() {
            break;
        }
        if hits[i].addr.0 >= region.end.0 {
            continue;
        }

        while i < hits.len() && hits[i].addr.0 < region.end.0 {
            let start = hits[i].addr.0;
            let want = ((region.end.0 - start) as usize).min(CHUNK);
            let n = match backend.read(Address(start), &mut buf[..want]) {
                Ok(n) if n > 0 => n,
                Err(e @ Error::Denied(_)) => return Err(e),
                // gone, so drop what is left in this region
                _ => {
                    while i < hits.len() && hits[i].addr.0 < region.end.0 {
                        i += 1;
                    }
                    break;
                }
            };

            let end = start + n as u64;
            while i < hits.len() && hits[i].addr.0 < end {
                let off = (hits[i].addr.0 - start) as usize;
                if off + size <= n {
                    let now = kind.decode(&buf[off..off + size]);
                    if keep(hits[i].prev, now) {
                        out.push(Hit {
                            addr: hits[i].addr,
                            prev: now,
                        });
                    }
                }
                i += 1;
            }
        }
    }

    Ok(out)
}

/// Narrow a set of hits with a keep-filter.
pub fn refine<B: MemoryBackend + ?Sized>(
    backend: &B,
    regions: &[Region],
    kind: ValueType,
    filter: Filter,
    hits: &[Hit],
) -> Result<Vec<Hit>, Error> {
    reread(backend, regions, kind, hits, &|prev, now| {
        filter.keeps(kind, prev, now)
    })
}

/// Refresh every hit's value in place, keeping them all. This is what live
/// polling uses, so the list shows current numbers instead of the scan-time ones.
pub fn refresh<B: MemoryBackend + ?Sized>(
    backend: &B,
    regions: &[Region],
    kind: ValueType,
    hits: &[Hit],
) -> Result<Vec<Hit>, Error> {
    reread(backend, regions, kind, hits, &|_, _| true)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::mock::MockBackend;
    use crate::procfs::RemoteProcess;

    fn writable(b: &MockBackend) -> Vec<Region> {
        b.regions().unwrap()
    }

    #[test]
    fn finds_an_exact_i32() {
        let b = MockBackend::new(0x100);
        b.write(Address(0x1020), &42i32.to_le_bytes()).unwrap();

        let hits = first(&b, &writable(&b), ValueType::I32, 42, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].addr, Address(0x1020));
        assert_eq!(hits[0].prev, 42);
    }

    #[test]
    fn finds_a_value_across_a_chunk_edge() {
        let b = MockBackend::new(CHUNK + 0x100);
        let at = Address(0x1000 + CHUNK as u64 - 2);
        b.write(at, &[0xaa, 0xbb, 0xcc, 0xdd]).unwrap();

        let bits = u32::from_le_bytes([0xaa, 0xbb, 0xcc, 0xdd]) as u64;
        let hits = first(&b, &writable(&b), ValueType::U32, bits, None).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].addr, at);
    }

    #[test]
    fn skips_read_only_regions() {
        let b = MockBackend::new(0x100);
        b.write(Address(0x1010), &7i32.to_le_bytes()).unwrap();

        let ro = vec![Region {
            start: Address(0x1000),
            end: Address(0x1100),
            read: true,
            write: false,
            exec: false,
            path: None,
        }];

        assert!(first(&b, &ro, ValueType::I32, 7, None).unwrap().is_empty());
    }

    #[test]
    fn max_stops_early() {
        let b = MockBackend::new(0x40);
        // the whole mock is zero, so every i32 offset is a hit
        let hits = first(&b, &writable(&b), ValueType::I32, 0, Some(3)).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn signedness_matters_when_comparing() {
        let minus_one = (-1i32) as u32 as u64;
        assert_eq!(ValueType::I32.cmp(minus_one, 1), Ordering::Less);
        assert_eq!(ValueType::U32.cmp(minus_one, 1), Ordering::Greater);
    }

    #[test]
    fn refine_changed_and_unchanged() {
        let b = MockBackend::new(0x100);
        b.write(Address(0x1010), &5i32.to_le_bytes()).unwrap();

        let regions = writable(&b);
        let hits = first(&b, &regions, ValueType::I32, 5, None).unwrap();
        assert_eq!(hits.len(), 1);

        b.write(Address(0x1010), &6i32.to_le_bytes()).unwrap();

        let changed = refine(&b, &regions, ValueType::I32, Filter::Changed, &hits).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].prev, 6);

        let unchanged = refine(&b, &regions, ValueType::I32, Filter::Unchanged, &hits).unwrap();
        assert!(unchanged.is_empty());
    }

    #[test]
    fn refine_increased_and_decreased() {
        let b = MockBackend::new(0x100);
        b.write(Address(0x1010), &5i32.to_le_bytes()).unwrap();
        b.write(Address(0x1020), &5i32.to_le_bytes()).unwrap();

        let regions = writable(&b);
        let hits = first(&b, &regions, ValueType::I32, 5, None).unwrap();
        assert_eq!(hits.len(), 2);

        b.write(Address(0x1010), &9i32.to_le_bytes()).unwrap();
        b.write(Address(0x1020), &2i32.to_le_bytes()).unwrap();

        let up = refine(&b, &regions, ValueType::I32, Filter::Increased, &hits).unwrap();
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].addr, Address(0x1010));

        let down = refine(&b, &regions, ValueType::I32, Filter::Decreased, &hits).unwrap();
        assert_eq!(down.len(), 1);
        assert_eq!(down[0].addr, Address(0x1020));
    }

    #[test]
    fn refine_exact_and_dropping_dead_hits() {
        let b = MockBackend::new(CHUNK + 0x100);
        b.write(Address(0x1010), &5i32.to_le_bytes()).unwrap();

        let regions = writable(&b);
        let hits = first(&b, &regions, ValueType::I32, 5, None).unwrap();

        b.write(Address(0x1010), &8i32.to_le_bytes()).unwrap();
        let kept = refine(&b, &regions, ValueType::I32, Filter::Exact(8), &hits).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].prev, 8);

        let missing = refine(&b, &regions, ValueType::I32, Filter::Exact(99), &hits).unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn refresh_updates_values_and_keeps_all() {
        let b = MockBackend::new(0x100);
        b.write(Address(0x1010), &5i32.to_le_bytes()).unwrap();
        b.write(Address(0x1020), &5i32.to_le_bytes()).unwrap();

        let regions = writable(&b);
        let hits = first(&b, &regions, ValueType::I32, 5, None).unwrap();
        assert_eq!(hits.len(), 2);

        b.write(Address(0x1010), &9i32.to_le_bytes()).unwrap();
        b.write(Address(0x1020), &2i32.to_le_bytes()).unwrap();

        let now = refresh(&b, &regions, ValueType::I32, &hits).unwrap();
        assert_eq!(now.len(), 2);
        assert_eq!(now[0].prev, 9);
        assert_eq!(now[1].prev, 2);
    }

    #[test]
    fn scans_and_refines_a_live_child() {
        let mut child = Command::new("sleep").arg("2").spawn().unwrap();
        let pid = child.id();
        wait_until_running(pid);

        let proc = RemoteProcess::open(pid).unwrap();
        let regions = proc.regions().unwrap();

        // same writable region the selftest pokes, and it exists under yama
        let Some(target) = regions
            .iter()
            .find(|r| r.read && r.write && r.path.is_some())
        else {
            child.kill().ok();
            child.wait().ok();
            return;
        };

        let magic: u64 = 0x1122_3344_5566_7788;
        // some sandboxes block ptrace outright, nothing to assert then
        if proc.write(target.start, &magic.to_le_bytes()).is_err() {
            child.kill().ok();
            child.wait().ok();
            return;
        }

        let hits = first(&proc, &regions, ValueType::U64, magic, None).unwrap();
        assert!(
            hits.iter().any(|h| h.addr == target.start),
            "magic value not found where it was written"
        );

        // change it, then the keep filter should follow it
        let other: u64 = 0x99aa_bbcc_ddee_ff00;
        proc.write(target.start, &other.to_le_bytes()).unwrap();
        let kept = refine(&proc, &regions, ValueType::U64, Filter::Exact(other), &hits).unwrap();
        assert!(kept.iter().any(|h| h.addr == target.start));

        child.kill().ok();
        child.wait().ok();
    }

    struct DeniedBackend;

    impl MemoryBackend for DeniedBackend {
        fn regions(&self) -> Result<Vec<Region>, Error> {
            Ok(vec![Region {
                start: Address(0x1000),
                end: Address(0x2000),
                read: true,
                write: true,
                exec: false,
                path: None,
            }])
        }

        fn read(&self, _: Address, _: &mut [u8]) -> Result<usize, Error> {
            Err(Error::Denied("nope".to_string()))
        }

        fn write(&self, _: Address, _: &[u8]) -> Result<usize, Error> {
            Err(Error::Denied("nope".to_string()))
        }
    }

    #[test]
    fn a_permission_wall_is_not_a_silent_zero() {
        let regions = DeniedBackend.regions().unwrap();
        let err = first(&DeniedBackend, &regions, ValueType::I32, 1, None).unwrap_err();
        assert!(matches!(err, Error::Denied(_)));
    }

    // spawn hands back before the child has replaced its image, so wait until
    // it is really running before trusting a read of its maps
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
}
