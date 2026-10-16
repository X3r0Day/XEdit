use xedit_core::{Error, Region};
use xedit_engine::process::{self, Proc};
use xedit_xtl::request::Request;

use crate::policy::{Capability, Policy};

pub enum Reply {
    Ok,
    Processes(Vec<Proc>),
    Regions(Vec<Region>),
}

/// The only way in. Everything, including our own screen, comes through here.
pub struct Desk {
    policy: Policy,
}

impl Desk {
    pub fn new(policy: Policy) -> Self {
        Desk { policy }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn handle(&self, req: Request) -> Result<Reply, Error> {
        match req {
            Request::Hello { .. } => Ok(Reply::Ok),

            Request::ListProcesses => Ok(Reply::Processes(process::list())),

            Request::Regions { pid } => Ok(Reply::Regions(process::regions(pid)?)),

            Request::Read { .. } => {
                self.need(Capability::ReadMemory)?;
                Err(Error::NotAttached) // wired up at M1
            }

            Request::Write { .. } => {
                self.need(Capability::WriteMemory)?;
                Err(Error::NotAttached)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_gated() {
        let desk = Desk::new(Policy::none());
        let r = desk.handle(Request::Read { addr: 0, len: 1 });
        assert!(matches!(r, Err(Error::Denied(_))));
    }

    #[test]
    fn listing_needs_nothing() {
        let desk = Desk::new(Policy::none());
        assert!(desk.handle(Request::ListProcesses).is_ok());
    }
}
