use crate::Address;

pub struct Edit {
    pub addr: Address,
    pub old: Vec<u8>,
    pub new: Vec<u8>,
}

#[derive(Default)]
pub struct Journal {
    entries: Vec<Edit>,
}

impl Journal {
    pub fn record(&mut self, addr: Address, old: Vec<u8>, new: Vec<u8>) {
        self.entries.push(Edit { addr, old, new });
    }

    pub fn last(&self) -> Option<&Edit> {
        self.entries.last()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_last_edit() {
        let mut j = Journal::default();
        assert!(j.is_empty());

        j.record(Address(0x1000), vec![0], vec![1]);
        j.record(Address(0x2000), vec![9], vec![8]);

        assert_eq!(j.len(), 2);
        assert_eq!(j.last().unwrap().addr, Address(0x2000));
    }
}
