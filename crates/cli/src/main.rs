use std::fs;
use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use xedit_core::Error;
use xedit_engine::scan::ValueType;
use xedit_host::desk::{Desk, Reply};
use xedit_host::policy::{Capability, Policy};
use xedit_xtl::request::Request;
use xedit_xtl::Hit;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("-V") | Some("--version") => {
            println!("xedit {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("tui") => run_tui(),
        Some("regions") => regions(&args),
        Some("peek") => peek(&args),
        Some("poke") => poke(&args),
        Some("scan") => scan(&args),
        Some("selftest") => selftest(),
        None => run_tui(),
        Some(_) => {
            eprintln!("usage: xedit [tui | regions <pid> | peek <pid> <addr> <len> | poke <pid> <addr> <hex> --yes | scan <pid> <type> <value> | selftest>");
            ExitCode::FAILURE
        }
    }
}

fn reader() -> Desk {
    Desk::new(Policy::none().allow(Capability::ReadMemory))
}

fn run_tui() -> ExitCode {
    let desk = Desk::new(
        Policy::none()
            .allow(Capability::ReadMemory)
            .allow(Capability::WriteMemory),
    );

    match xedit_tui::run(desk) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tui: {e}");
            ExitCode::FAILURE
        }
    }
}

fn regions(args: &[String]) -> ExitCode {
    let Some(pid) = args.get(1).and_then(|s| s.parse::<u32>().ok()) else {
        return usage("xedit regions <pid>");
    };
    show(reader().handle(Request::Regions { pid }))
}

fn peek(args: &[String]) -> ExitCode {
    let (Some(pid), Some(addr), Some(len)) = (
        args.get(1).and_then(|s| s.parse::<u32>().ok()),
        args.get(2).and_then(|s| parse_addr(s)),
        args.get(3).and_then(|s| s.parse::<u64>().ok()),
    ) else {
        return usage("xedit peek <pid> <addr> <len>");
    };

    let mut desk = reader();
    if let Err(e) = desk.handle(Request::Attach { pid }) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }

    match desk.handle(Request::Read { addr, len }) {
        Ok(Reply::Bytes(bytes)) => {
            hexdump(addr, &bytes);
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn poke(args: &[String]) -> ExitCode {
    let (Some(pid), Some(addr), Some(bytes)) = (
        args.get(1).and_then(|s| s.parse::<u32>().ok()),
        args.get(2).and_then(|s| parse_addr(s)),
        args.get(3).and_then(|s| parse_bytes(s)),
    ) else {
        return usage("xedit poke <pid> <addr> <hex bytes> --yes");
    };

    if !args.iter().any(|a| a == "--yes" || a == "-y") {
        eprintln!(
            "about to write {} byte(s) to pid {pid} at 0x{addr:x}",
            bytes.len()
        );
        eprintln!("that changes another program's memory, re-run with --yes if you mean it");
        return ExitCode::FAILURE;
    }

    let mut desk = Desk::new(
        Policy::none()
            .allow(Capability::ReadMemory)
            .allow(Capability::WriteMemory),
    );

    if let Err(e) = desk.handle(Request::Attach { pid }) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }

    match desk.handle(Request::Write { addr, bytes }) {
        Ok(Reply::Wrote { addr, old, new }) => {
            println!("wrote {} byte(s) at 0x{:x}", new.len(), addr.0);
            println!("old: {}", hex(&old));
            println!("new: {}", hex(&new));
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

const SCAN_USAGE: &str = "\
xedit scan <pid> <type> <value> [--max N]   start a scan
xedit scan <pid> <filter> [value]           narrow it, filter is one of
                                            changed, unchanged, increased,
                                            decreased, exact
xedit scan <pid>                            show the current matches
xedit scan <pid> clear                      forget the saved scan";

fn scan(args: &[String]) -> ExitCode {
    let Some(pid) = args.get(1).and_then(|s| s.parse::<u32>().ok()) else {
        return usage(SCAN_USAGE);
    };

    match args.get(2).map(String::as_str) {
        None => show_scan(pid),
        Some("clear") => clear_scan(pid),
        Some(name) if ValueType::parse(name).is_some() => first_scan(args, pid),
        Some(name) if is_filter(name) => refine_scan(args, pid),
        Some(_) => usage(SCAN_USAGE),
    }
}

fn is_filter(name: &str) -> bool {
    matches!(
        name,
        "changed" | "unchanged" | "increased" | "decreased" | "exact"
    )
}

fn first_scan(args: &[String], pid: u32) -> ExitCode {
    let kind = args[2].as_str();
    let (Some(value), Some(kind_type)) = (args.get(3), ValueType::parse(kind)) else {
        return usage(SCAN_USAGE);
    };

    let max = match parse_max(args) {
        Some(max) => max,
        None => return usage(SCAN_USAGE),
    };

    let mut desk = reader();
    if let Err(e) = desk.handle(Request::Attach { pid }) {
        return fail(e);
    }

    let hits = match desk.handle(Request::Scan {
        kind: kind.to_string(),
        value: value.clone(),
        max,
    }) {
        Ok(Reply::Matches(hits)) => hits,
        Ok(_) => return ExitCode::FAILURE,
        Err(e) => return fail(e),
    };

    let session = Session {
        pid,
        kind: kind.to_string(),
        hits,
    };
    if let Err(e) = save(&session, &session_path()) {
        eprintln!("couldn't save the scan: {e}");
        return ExitCode::FAILURE;
    }

    print_matches(pid, kind_type, &session.hits)
}

fn refine_scan(args: &[String], pid: u32) -> ExitCode {
    let filter = args[2].as_str();
    let value = args.get(3).cloned().unwrap_or_default();

    let session = match load(&session_path()) {
        Ok(Some(s)) if s.pid == pid => s,
        Ok(Some(s)) => {
            eprintln!(
                "the saved scan is for pid {}, start a new one with `xedit scan {pid} <type> <value>`",
                s.pid
            );
            return ExitCode::FAILURE;
        }
        Ok(None) => {
            eprintln!("no saved scan, start one with `xedit scan {pid} <type> <value>`");
            return ExitCode::FAILURE;
        }
        Err(e) => return fail(e.into()),
    };

    let Some(kind) = ValueType::parse(&session.kind) else {
        eprintln!("the saved scan has an unknown type {:?}", session.kind);
        return ExitCode::FAILURE;
    };

    let mut desk = reader();
    if let Err(e) = desk.handle(Request::Attach { pid }) {
        return fail(e);
    }

    let hits = match desk.handle(Request::Refine {
        kind: session.kind.clone(),
        filter: filter.to_string(),
        value,
        hits: session.hits,
    }) {
        Ok(Reply::Matches(hits)) => hits,
        Ok(_) => return ExitCode::FAILURE,
        Err(e) => return fail(e),
    };

    if let Err(e) = save(
        &Session {
            pid,
            kind: session.kind,
            hits: hits.clone(),
        },
        &session_path(),
    ) {
        eprintln!("couldn't save the scan: {e}");
        return ExitCode::FAILURE;
    }

    print_matches(pid, kind, &hits)
}

fn show_scan(pid: u32) -> ExitCode {
    match load(&session_path()) {
        Ok(Some(s)) if s.pid == pid => {
            let kind = ValueType::parse(&s.kind).unwrap_or(ValueType::U32);
            print_matches(pid, kind, &s.hits)
        }
        Ok(Some(s)) => {
            eprintln!("the saved scan is for pid {}", s.pid);
            ExitCode::FAILURE
        }
        Ok(None) => {
            eprintln!("no saved scan");
            ExitCode::FAILURE
        }
        Err(e) => fail(e.into()),
    }
}

fn clear_scan(pid: u32) -> ExitCode {
    let path = session_path();
    if path.exists() {
        if let Err(e) = fs::remove_file(&path) {
            eprintln!("couldn't clear the scan: {e}");
            return ExitCode::FAILURE;
        }
    }
    println!("cleared scan for pid {pid}");
    ExitCode::SUCCESS
}

fn parse_max(args: &[String]) -> Option<Option<u64>> {
    for (i, a) in args.iter().enumerate() {
        if a == "--max" || a == "-m" {
            return Some(Some(args.get(i + 1)?.parse().ok()?));
        }
    }
    Some(None)
}

fn print_matches(pid: u32, kind: ValueType, hits: &[Hit]) -> ExitCode {
    println!(
        "{pid}: {} match{} ({})",
        hits.len(),
        if hits.len() == 1 { "" } else { "es" },
        kind.name()
    );

    for h in hits.iter().take(PREVIEW) {
        println!("0x{:012x}  {}", h.addr, kind.format(h.prev));
    }
    if hits.len() > PREVIEW {
        println!("... {} more", hits.len() - PREVIEW);
    }

    ExitCode::SUCCESS
}

const SESSION_MAGIC: &[u8; 4] = b"XESC";
const SESSION_VERSION: u8 = 1;
const PREVIEW: usize = 20;

struct Session {
    pid: u32,
    kind: String,
    hits: Vec<Hit>,
}

fn session_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".cache")
        });
    base.join("xedit").join("scan.bin")
}

// a compact little-endian file, since a scan set can be large
fn save(session: &Session, path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }

    let mut out = io::BufWriter::new(fs::File::create(path)?);
    out.write_all(SESSION_MAGIC)?;
    out.write_all(&[SESSION_VERSION])?;
    out.write_all(&session.pid.to_le_bytes())?;
    out.write_all(&[session.kind.len() as u8])?;
    out.write_all(session.kind.as_bytes())?;
    out.write_all(&(session.hits.len() as u64).to_le_bytes())?;

    for h in &session.hits {
        out.write_all(&h.addr.to_le_bytes())?;
        out.write_all(&h.prev.to_le_bytes())?;
    }

    out.flush()
}

fn load(path: &Path) -> io::Result<Option<Session>> {
    let Ok(mut file) = fs::File::open(path) else {
        return Ok(None);
    };

    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() || magic != *SESSION_MAGIC {
        return Ok(None);
    }

    // anything past the magic that does not parse means the file is unusable,
    // worth treating as "no saved scan" rather than an error
    Ok(read_body(&mut file).ok())
}

fn read_body(file: &mut fs::File) -> io::Result<Session> {
    let mut version = [0u8; 1];
    file.read_exact(&mut version)?;
    if version[0] != SESSION_VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "version"));
    }

    let mut pid = [0u8; 4];
    file.read_exact(&mut pid)?;

    let mut klen = [0u8; 1];
    file.read_exact(&mut klen)?;
    let mut kind = vec![0u8; klen[0] as usize];
    file.read_exact(&mut kind)?;

    let mut count = [0u8; 8];
    file.read_exact(&mut count)?;
    let count = u64::from_le_bytes(count) as usize;

    // never trust the count for the allocation, a corrupt file is not an OOM
    let mut hits = Vec::with_capacity(count.min(1 << 20));
    let mut pair = [0u8; 16];
    for _ in 0..count {
        file.read_exact(&mut pair)?;
        hits.push(Hit {
            addr: u64::from_le_bytes(pair[..8].try_into().unwrap()),
            prev: u64::from_le_bytes(pair[8..].try_into().unwrap()),
        });
    }

    Ok(Session {
        pid: u32::from_le_bytes(pid),
        kind: String::from_utf8_lossy(&kind).into_owned(),
        hits,
    })
}

fn selftest() -> ExitCode {
    let mut child = match Command::new("sleep").arg("2").spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("couldn't spawn sleep: {e}");
            return ExitCode::FAILURE;
        }
    };

    let pid = child.id();
    println!("spawned sleep as pid {pid}");
    wait_for_ready(pid);

    let code = probe(pid);

    child.kill().ok();
    child.wait().ok();
    code
}

// spawn returns before the child has replaced its image, and the dynamic loader
// is still mprotecting segments for a moment after that, so wait until the
// process is actually running before trusting anything out of its maps
fn wait_for_ready(pid: u32) {
    let stat = format!("/proc/{pid}/stat");
    for _ in 0..200 {
        if let Ok(s) = std::fs::read_to_string(&stat) {
            // "pid (name) S ..." and the name can hold anything, so skip past the last ')'
            if let Some((_, rest)) = s.rsplit_once(')') {
                if rest.trim_start().starts_with('S') {
                    return;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

// exercises attach, read and write against a child process, which works even
// when yama blocks tracing everything else
fn probe(pid: u32) -> ExitCode {
    let mut desk = Desk::new(
        Policy::none()
            .allow(Capability::ReadMemory)
            .allow(Capability::WriteMemory),
    );

    if let Err(e) = desk.handle(Request::Attach { pid }) {
        return fail(e);
    }

    let regions = match desk.handle(Request::Regions { pid }) {
        Ok(Reply::Regions(r)) => r,
        Ok(_) => return ExitCode::FAILURE,
        Err(e) => return fail(e),
    };

    let Some(readable) = regions.iter().find(|r| r.read && r.path.is_some()) else {
        eprintln!("no readable region");
        return ExitCode::FAILURE;
    };

    println!("read {}", readable.path.as_deref().unwrap_or("?"));
    let bytes = match read(&mut desk, readable.start.0, 32) {
        Ok(bytes) => bytes,
        Err(e) => return fail(e),
    };
    hexdump(readable.start.0, &bytes);

    // flip a few bits in a writable region, check they landed, then put them back
    let Some(writable) = regions.iter().find(|r| r.write && r.path.is_some()) else {
        eprintln!("no writable region");
        return ExitCode::FAILURE;
    };
    let addr = writable.start.0;

    let before = match read(&mut desk, addr, 8) {
        Ok(bytes) => bytes,
        Err(e) => return fail(e),
    };

    let flipped: Vec<u8> = before.iter().map(|b| !b).collect();
    if let Err(e) = write(&mut desk, addr, &flipped) {
        return fail(e);
    }

    let mid = match read(&mut desk, addr, 8) {
        Ok(bytes) => bytes,
        Err(e) => return fail(e),
    };

    // put the original bytes back no matter how the check above went
    if let Err(e) = write(&mut desk, addr, &before) {
        return fail(e);
    }

    let after = match read(&mut desk, addr, 8) {
        Ok(bytes) => bytes,
        Err(e) => return fail(e),
    };

    println!("write {}", writable.path.as_deref().unwrap_or("?"));
    println!("  before  {}", hex(&before));
    println!("  flipped {}", hex(&mid));
    println!("  after   {}", hex(&after));
    let wrote_ok = mid == flipped && after == before;

    // and the scan path, on the same region
    let magic: u64 = 0x1122_3344_5566_7788;
    if let Err(e) = write(&mut desk, addr, &magic.to_le_bytes()) {
        return fail(e);
    }

    let hits = match desk.handle(Request::Scan {
        kind: "u64".to_string(),
        value: magic.to_string(),
        max: None,
    }) {
        Ok(Reply::Matches(hits)) => hits,
        Ok(_) => return ExitCode::FAILURE,
        Err(e) => return fail(e),
    };
    let scanned = hits.iter().any(|h| h.addr == addr);

    let other: u64 = 0x99aa_bbcc_ddee_ff00;
    if let Err(e) = write(&mut desk, addr, &other.to_le_bytes()) {
        return fail(e);
    }
    let refined = match desk.handle(Request::Refine {
        kind: "u64".to_string(),
        filter: "exact".to_string(),
        value: other.to_string(),
        hits,
    }) {
        Ok(Reply::Matches(hits)) => hits.iter().any(|h| h.addr == addr),
        Ok(_) => false,
        Err(e) => return fail(e),
    };

    // put the original bytes back
    if let Err(e) = write(&mut desk, addr, &before) {
        return fail(e);
    }

    println!("scan   {}", if scanned { "found it" } else { "missed" });
    println!("refine {}", if refined { "followed it" } else { "missed" });

    if wrote_ok && scanned && refined {
        println!("ok");
        ExitCode::SUCCESS
    } else {
        eprintln!("selftest failed");
        ExitCode::FAILURE
    }
}

fn read(desk: &mut Desk, addr: u64, len: u64) -> Result<Vec<u8>, Error> {
    match desk.handle(Request::Read { addr, len })? {
        Reply::Bytes(bytes) => Ok(bytes),
        _ => Ok(Vec::new()),
    }
}

fn write(desk: &mut Desk, addr: u64, bytes: &[u8]) -> Result<(), Error> {
    desk.handle(Request::Write {
        addr,
        bytes: bytes.to_vec(),
    })?;
    Ok(())
}

fn fail(e: Error) -> ExitCode {
    eprintln!("{e}");
    ExitCode::FAILURE
}

fn show(reply: Result<Reply, Error>) -> ExitCode {
    match reply {
        Ok(Reply::Regions(regions)) => emit(regions.iter().map(|r| r.to_string())),
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn emit(lines: impl Iterator<Item = String>) -> ExitCode {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in lines {
        // `xedit list | head` closes the pipe on us, which is not an error
        if let Err(e) = writeln!(out, "{line}") {
            if e.kind() == io::ErrorKind::BrokenPipe {
                return ExitCode::SUCCESS;
            }
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}

fn hexdump(base: u64, bytes: &[u8]) {
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let addr = base + (i * 16) as u64;
        let mut details = String::with_capacity(49);
        let mut text = String::with_capacity(16);

        for (j, b) in chunk.iter().enumerate() {
            if j == 8 {
                details.push(' ');
            }
            details.push_str(&format!("{b:02x} "));
            text.push(if (0x20..0x7f).contains(b) {
                *b as char
            } else {
                '.'
            });
        }

        println!("0x{addr:012x}  {details:<49} {text}");
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_addr(s: &str) -> Option<u64> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

fn parse_bytes(s: &str) -> Option<Vec<u8>> {
    let clean: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '_' && *c != ':')
        .collect();
    let clean = clean.strip_prefix("0x").unwrap_or(&clean);

    if clean.is_empty() || !clean.len().is_multiple_of(2) {
        return None;
    }

    let d = clean.as_bytes();
    let mut out = Vec::with_capacity(clean.len() / 2);
    for i in (0..d.len()).step_by(2) {
        let hi = (d[i] as char).to_digit(16)?;
        let lo = (d[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("usage: {msg}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("xedit-{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn session_round_trips() {
        let path = temp_path("scan");
        let session = Session {
            pid: 4321,
            kind: "i32".to_string(),
            hits: vec![
                Hit {
                    addr: 0x1000,
                    prev: 7,
                },
                Hit {
                    addr: 0x2000,
                    prev: 9,
                },
            ],
        };

        save(&session, &path).unwrap();
        let back = load(&path).unwrap().unwrap();

        assert_eq!(back.pid, 4321);
        assert_eq!(back.kind, "i32");
        assert_eq!(back.hits, session.hits);

        fs::remove_file(&path).ok();
    }

    #[test]
    fn a_missing_session_is_none() {
        assert!(load(&temp_path("nope")).unwrap().is_none());
    }

    #[test]
    fn a_truncated_session_is_none() {
        let path = temp_path("short");
        fs::write(&path, b"XESC").unwrap();
        assert!(load(&path).unwrap().is_none());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn only_known_filters_are_accepted() {
        assert!(is_filter("changed"));
        assert!(is_filter("exact"));
        assert!(!is_filter("whatever"));
    }
}
