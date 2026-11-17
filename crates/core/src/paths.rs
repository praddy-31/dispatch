//! Where Dispatch keeps its files, and atomic writes.

use std::path::{Path, PathBuf};

/// `$DISPATCH_HOME` if set (tests and manual runs use a fake one), otherwise
/// `~/Library/Application Support/Dispatch`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DISPATCH_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    home().join("Library/Application Support/Dispatch")
}

/// Palette's current theme folder. `$DISPATCH_PALETTE_DIR` overrides it in tests.
pub fn palette_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DISPATCH_PALETTE_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    home().join(".config/palette/current")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Writes to a temp file beside `path`, then renames it over `path`, so a crash never
/// leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("no parent folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let name = path.file_name().ok_or("no file name")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("can't replace {}: {e}", path.display())
    })
}
