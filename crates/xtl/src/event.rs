use serde::{Deserialize, Serialize};

/// Things the host tells a client about, without being asked.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Attached { pid: u32 },
    Detached { pid: u32 },
    WriteApplied { addr: u64, len: u64 },
}
