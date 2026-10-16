use std::fs;
use std::path::{Path, PathBuf};

pub struct Discovered {
    pub dir: PathBuf,
    pub manifest: PathBuf,
}

/// Look for add-on folders: a directory with an addon.toml inside it.
pub fn discover(root: &Path) -> Vec<Discovered> {
    let mut out = Vec::new();

    let Ok(entries) = fs::read_dir(root) else {
        return out;
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        let manifest = dir.join("addon.toml");
        if dir.is_dir() && manifest.is_file() {
            out.push(Discovered { dir, manifest });
        }
    }

    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}
