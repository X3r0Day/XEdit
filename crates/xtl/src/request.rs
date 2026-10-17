use serde::{Deserialize, Serialize};

use crate::hit::Hit;

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
    Attach {
        pid: u32,
    },
    Detach,
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
    Scan {
        kind: String,
        value: String,
        max: Option<u64>,
    },
    Refine {
        kind: String,
        filter: String,
        value: String,
        hits: Vec<Hit>,
    },
    Refresh {
        kind: String,
        hits: Vec<Hit>,
    },
    Pin {
        addr: u64,
        bytes: Vec<u8>,
    },
}
