#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    ReadMemory,
    ScanMemory,
    WriteMemory,
    Internet,
    AddPanel,
}

impl Capability {
    pub fn name(self) -> &'static str {
        match self {
            Capability::ReadMemory => "read memory",
            Capability::ScanMemory => "scan memory",
            Capability::WriteMemory => "write memory",
            Capability::Internet => "the internet",
            Capability::AddPanel => "add a panel",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Policy {
    granted: Vec<Capability>,
}

impl Policy {
    pub fn none() -> Self {
        Policy::default()
    }

    pub fn allow(mut self, cap: Capability) -> Self {
        if !self.granted.contains(&cap) {
            self.granted.push(cap);
        }
        self
    }

    pub fn allows(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_grants_what_was_asked() {
        let p = Policy::none().allow(Capability::ReadMemory);
        assert!(p.allows(Capability::ReadMemory));
        assert!(!p.allows(Capability::WriteMemory));
    }
}
