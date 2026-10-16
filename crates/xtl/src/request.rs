use serde::{Deserialize, Serialize};

/// Every front desk method. One variant per thing a client can ask for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Hello {
        id: String,
        api: String,
        caps: Vec<String>,
    },
    ListProcesses,
    Regions {
        pid: u32,
    },
    Read {
        addr: u64,
        len: u64,
    },
    Write {
        addr: u64,
        bytes: Vec<u8>,
    },
}
