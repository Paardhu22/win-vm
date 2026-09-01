//! Minimal `which(1)`: locate an executable on `PATH`.
//!
//! Implemented here rather than pulled in as a dependency because the whole
//! behaviour is a dozen lines and preflight needs the *path* it resolved to,
//! so it can tell the user which binary was actually picked up.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Resolve `name` against the current process `PATH`.
pub fn find(name: &str) -> Option<PathBuf> {
    find_in(name, &std::env::var_os("PATH")?)
}

/// Resolve `name` against an explicit `PATH` value.
pub fn find_in(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("daholyvm-which-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_an_executable_and_ignores_a_non_executable_of_the_same_name() {
        let shadow = scratch("shadow");
        let real = scratch("real");
        fs::write(shadow.join("tool"), "").unwrap();
        fs::write(real.join("tool"), "").unwrap();
        fs::set_permissions(real.join("tool"), fs::Permissions::from_mode(0o755)).unwrap();

        let path = std::env::join_paths([&shadow, &real]).unwrap();
        assert_eq!(find_in("tool", &path), Some(real.join("tool")));
    }

    #[test]
    fn earlier_path_entry_wins() {
        let first = scratch("first");
        let second = scratch("second");
        for dir in [&first, &second] {
            fs::write(dir.join("tool"), "").unwrap();
            fs::set_permissions(dir.join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(find_in("tool", &path), Some(first.join("tool")));
    }

    #[test]
    fn missing_executable_is_none() {
        let dir = scratch("empty");
        let path = std::env::join_paths([&dir]).unwrap();
        assert_eq!(find_in("tool", &path), None);
    }
}
