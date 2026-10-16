use xedit_core::{Address, Error, Region};

/// Everything that can read and write memory implements this. The real backend
/// talks to another process; the mock one is just a byte buffer, which is how
/// the engine gets tested without ptrace.
pub trait MemoryBackend {
    fn regions(&self) -> Result<Vec<Region>, Error>;

    fn read(&self, addr: Address, buf: &mut [u8]) -> Result<usize, Error>;

    fn write(&self, addr: Address, bytes: &[u8]) -> Result<usize, Error>;
}
