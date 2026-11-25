//! Shared between the client and the daemon.

use std::path::PathBuf;

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("cliphist-picker.sock")
}
