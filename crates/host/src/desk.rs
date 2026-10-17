use xedit_core::journal::Journal;
use xedit_core::{Address, Error, Region};
use xedit_engine::process::{self, Proc};
use xedit_engine::procfs::RemoteProcess;
use xedit_engine::scan::{self, Filter, ValueType};
use xedit_engine::MemoryBackend;
use xedit_xtl::request::Request;
use xedit_xtl::Hit;

use crate::policy::{Capability, Policy};

/// one request may not ask for more than this, so an add-on can't make the host
/// allocate without bound
const MAX_READ: u64 = 16 << 20;

pub enum Reply {
    Ok,
    Processes(Vec<Proc>),
    Regions(Vec<Region>),
    Attached {
        pid: u32,
    },
    Bytes(Vec<u8>),
    Wrote {
        addr: Address,
        old: Vec<u8>,
        new: Vec<u8>,
    },
    Matches(Vec<Hit>),
}

/// The only way in. Everything, including our own screen, comes through here.
pub struct Desk {
    policy: Policy,
    attached: Option<RemoteProcess>,
    journal: Journal,
}

impl Desk {
    pub fn new(policy: Policy) -> Self {
        Desk {
            policy,
            attached: None,
            journal: Journal::default(),
        }
    }

    pub fn attached_pid(&self) -> Option<u32> {
        self.attached.as_ref().map(RemoteProcess::pid)
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn handle(&mut self, req: Request) -> Result<Reply, Error> {
        match req {
            Request::Hello { .. } => Ok(Reply::Ok),

            Request::ListProcesses => Ok(Reply::Processes(process::list())),

            Request::Attach { pid } => {
                self.need(Capability::ReadMemory)?;
                let proc = RemoteProcess::open(pid)?;
                let pid = proc.pid();
                self.attached = Some(proc);
                Ok(Reply::Attached { pid })
            }

            Request::Detach => {
                self.attached = None;
                Ok(Reply::Ok)
            }

            Request::Regions { pid } => Ok(Reply::Regions(process::regions(pid)?)),

            Request::Read { addr, len } => {
                self.need(Capability::ReadMemory)?;
                if len > MAX_READ {
                    return Err(Error::TooLarge(len));
                }

                let proc = self.attached.as_ref().ok_or(Error::NotAttached)?;

                let mut buf = vec![0u8; len as usize];
                let n = proc.read(Address(addr), &mut buf)?;
                buf.truncate(n);
                Ok(Reply::Bytes(buf))
            }

            Request::Write { addr, bytes } => {
                self.need(Capability::WriteMemory)?;
                let addr = Address(addr);
                let mut new = bytes;

                // grab the old bytes first so the write stays undoable
                let old = {
                    let proc = self.attached.as_ref().ok_or(Error::NotAttached)?;

                    let mut old = vec![0u8; new.len()];
                    let n = proc.read(addr, &mut old)?;
                    old.truncate(n);

                    let written = proc.write(addr, &new)?;
                    // a write can stop at a mapping edge, only claim what landed
                    new.truncate(written);
                    old.truncate(written);
                    old
                };

                self.journal.record(addr, old.clone(), new.clone());
                Ok(Reply::Wrote { addr, old, new })
            }

            Request::Scan { kind, value, max } => {
                self.need(Capability::ReadMemory)?;
                let kind = parse_kind(&kind)?;
                let bits = kind.parse_value(&value).ok_or_else(|| {
                    Error::BadRequest(format!("{value:?} is not a {}", kind.name()))
                })?;

                let proc = self.attached.as_ref().ok_or(Error::NotAttached)?;
                let regions = proc.regions()?;
                let hits = scan::first(proc, &regions, kind, bits, max.map(|m| m as usize))?;
                Ok(Reply::Matches(to_wire(hits)))
            }

            Request::Refine {
                kind,
                filter,
                value,
                hits,
            } => {
                self.need(Capability::ReadMemory)?;
                let kind = parse_kind(&kind)?;
                let filter = Filter::parse(kind, &filter, Some(&value))
                    .ok_or_else(|| Error::BadRequest(format!("unknown filter {filter:?}")))?;

                let proc = self.attached.as_ref().ok_or(Error::NotAttached)?;
                let regions = proc.regions()?;
                let prev = from_wire(hits);
                let kept = scan::refine(proc, &regions, kind, filter, &prev)?;
                Ok(Reply::Matches(to_wire(kept)))
            }

            Request::Refresh { kind, hits } => {
                self.need(Capability::ReadMemory)?;
                let kind = parse_kind(&kind)?;

                let proc = self.attached.as_ref().ok_or(Error::NotAttached)?;
                let regions = proc.regions()?;
                let prev = from_wire(hits);
                let now = scan::refresh(proc, &regions, kind, &prev)?;
                Ok(Reply::Matches(to_wire(now)))
            }

            // a freeze re-writes the same bytes over and over, so it skips the
            // journal the way a poke does not
            Request::Pin { addr, bytes } => {
                self.need(Capability::WriteMemory)?;
                let proc = self.attached.as_ref().ok_or(Error::NotAttached)?;
                proc.write(Address(addr), &bytes)?;
                Ok(Reply::Ok)
            }
        }
    }

    fn need(&self, cap: Capability) -> Result<(), Error> {
        if self.policy.allows(cap) {
            Ok(())
        } else {
            Err(Error::Denied(cap.name().to_string()))
        }
    }
}

fn parse_kind(name: &str) -> Result<ValueType, Error> {
    ValueType::parse(name).ok_or_else(|| Error::BadRequest(format!("unknown type {name:?}")))
}

fn to_wire(hits: Vec<scan::Hit>) -> Vec<Hit> {
    hits.into_iter()
        .map(|h| Hit {
            addr: h.addr.0,
            prev: h.prev,
        })
        .collect()
}

fn from_wire(hits: Vec<Hit>) -> Vec<scan::Hit> {
    hits.into_iter()
        .map(|h| scan::Hit {
            addr: Address(h.addr),
            prev: h.prev,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_gated() {
        let mut desk = Desk::new(Policy::none());
        let r = desk.handle(Request::Read { addr: 0, len: 1 });
        assert!(matches!(r, Err(Error::Denied(_))));
    }

    #[test]
    fn listing_needs_nothing() {
        let mut desk = Desk::new(Policy::none());
        assert!(desk.handle(Request::ListProcesses).is_ok());
    }

    #[test]
    fn write_without_attach() {
        let mut desk = Desk::new(Policy::none().allow(Capability::WriteMemory));
        let r = desk.handle(Request::Write {
            addr: 0,
            bytes: vec![1],
        });
        assert!(matches!(r, Err(Error::NotAttached)));
    }

    #[test]
    fn read_without_attach() {
        let mut desk = Desk::new(Policy::none().allow(Capability::ReadMemory));
        let r = desk.handle(Request::Read { addr: 0, len: 1 });
        assert!(matches!(r, Err(Error::NotAttached)));
    }

    #[test]
    fn oversized_reads_are_rejected() {
        let mut desk = Desk::new(Policy::none().allow(Capability::ReadMemory));
        let r = desk.handle(Request::Read {
            addr: 0,
            len: u64::MAX,
        });
        assert!(matches!(r, Err(Error::TooLarge(_))));
    }

    #[test]
    fn scanning_is_gated() {
        let mut desk = Desk::new(Policy::none());
        let r = desk.handle(Request::Scan {
            kind: "i32".to_string(),
            value: "1".to_string(),
            max: None,
        });
        assert!(matches!(r, Err(Error::Denied(_))));
    }

    #[test]
    fn scan_without_attach() {
        let mut desk = Desk::new(Policy::none().allow(Capability::ReadMemory));
        let r = desk.handle(Request::Scan {
            kind: "i32".to_string(),
            value: "1".to_string(),
            max: None,
        });
        assert!(matches!(r, Err(Error::NotAttached)));
    }

    #[test]
    fn refine_without_attach() {
        let mut desk = Desk::new(Policy::none().allow(Capability::ReadMemory));
        let r = desk.handle(Request::Refine {
            kind: "i32".to_string(),
            filter: "changed".to_string(),
            value: String::new(),
            hits: Vec::new(),
        });
        assert!(matches!(r, Err(Error::NotAttached)));
    }
}
