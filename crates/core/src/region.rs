use std::fmt;

use crate::address::Address;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: Address,
    pub end: Address,
    pub read: bool,
    pub write: bool,
    pub exec: bool,
    pub path: Option<String>,
}

impl Region {
    pub fn len(&self) -> u64 {
        self.end.0.saturating_sub(self.start.0)
    }

    pub fn is_empty(&self) -> bool {
        self.end.0 <= self.start.0
    }

    pub fn contains(&self, addr: Address) -> bool {
        addr.0 >= self.start.0 && addr.0 < self.end.0
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = if self.read { 'r' } else { '-' };
        let w = if self.write { 'w' } else { '-' };
        let x = if self.exec { 'x' } else { '-' };
        write!(
            f,
            "{} - {} {}{}{} {}",
            self.start,
            self.end,
            r,
            w,
            x,
            self.path.as_deref().unwrap_or("[anon]")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> Region {
        Region {
            start: Address(0x1000),
            end: Address(0x2000),
            read: true,
            write: true,
            exec: false,
            path: None,
        }
    }

    #[test]
    fn contains_is_half_open() {
        let r = region();
        assert!(r.contains(Address(0x1000)));
        assert!(r.contains(Address(0x1fff)));
        assert!(!r.contains(Address(0x2000)));
        assert!(!r.contains(Address(0x0fff)));
    }

    #[test]
    fn len() {
        assert_eq!(region().len(), 0x1000);
    }
}
