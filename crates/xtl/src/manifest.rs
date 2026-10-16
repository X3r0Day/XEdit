use serde::Deserialize;

/// The `addon.toml` a runtime addon ships with.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub addon: AddonInfo,
    pub run: Run,
    #[serde(default)]
    pub asks_for: AsksFor,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddonInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub program: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AsksFor {
    #[serde(default)]
    pub keys: Vec<String>,
}
