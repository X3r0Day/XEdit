use serde::{Deserialize, Serialize};

/// One address a scan turned up, and the value it held when we last looked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub addr: u64,
    pub prev: u64,
}
