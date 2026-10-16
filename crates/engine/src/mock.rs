use std::cell::RefCell;

use xedit_core::{Address, Error, Region};

use crate::backend::MemoryBackend;

/// A stand-in for a real process. The trait hands out `&self` because the real
/// backend is just syscalls, so the bytes live behind a RefCell here.
pub struct MockBackend {
    base: Address,
    mem: RefCell<Vec<u8>>,
}

impl MockBackend {
    pub fn new(size: usize) -> Self {
        MockBackend {
            base: Address(0x1000),
            mem: RefCell::new(vec![0; size]),
        }
    }

    fn offset(&self, addr: Address) -> Result<usize, Error> {
        addr.0
            .checked_sub(self.base.0)
            .map(|o| o as usize)
            .ok_or(Error::Unmapped(addr.0))
    }
}

impl MemoryBackend for MockBackend {
    fn regions(&self) -> Result<Vec<Region>, Error> {
        Ok(vec![Region {
            start: self.base,
            end: Address(self.base.0 + self.mem.borrow().len() as u64),
            read: true,
            write: true,
            exec: false,
            path: None,
        }])
    }

    fn read(&self, addr: Address, buf: &mut [u8]) -> Result<usize, Error> {
        let off = self.offset(addr)?;
        let mem = self.mem.borrow();
        if off >= mem.len() {
            return Err(Error::Unmapped(addr.0));
        }
        let n = buf.len().min(mem.len() - off);
        buf[..n].copy_from_slice(&mem[off..off + n]);
        Ok(n)
    }

    fn write(&self, addr: Address, bytes: &[u8]) -> Result<usize, Error> {
        let off = self.offset(addr)?;
        let mut mem = self.mem.borrow_mut();
        if off >= mem.len() {
            return Err(Error::Unmapped(addr.0));
        }
        let n = bytes.len().min(mem.len() - off);
        mem[off..off + n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read() {
        let b = MockBackend::new(16);
        b.write(Address(0x1004), &[1, 2, 3, 4]).unwrap();

        let mut buf = [0u8; 4];
        assert_eq!(b.read(Address(0x1004), &mut buf).unwrap(), 4);
        assert_eq!(buf, [1, 2, 3, 4]);
    }

    #[test]
    fn read_past_end_is_clamped() {
        let b = MockBackend::new(8);
        let mut buf = [0xffu8; 4];
        // only 4 bytes left from base+4, so we get 4
        assert_eq!(b.read(Address(0x1004), &mut buf).unwrap(), 4);

        let mut buf2 = [0u8; 4];
        assert_eq!(b.read(Address(0x1006), &mut buf2).unwrap(), 2);
    }

    #[test]
    fn below_base_is_unmapped() {
        let b = MockBackend::new(8);
        let mut buf = [0u8; 1];
        assert!(matches!(
            b.read(Address(0x10), &mut buf),
            Err(Error::Unmapped(_))
        ));
    }
}
