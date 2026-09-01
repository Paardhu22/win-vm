//! Virtual disk images, created through `qemu-img`.
//!
//! qcow2 is used for every disk: it allocates lazily, so a 64 GiB Windows disk
//! costs a few megabytes until the guest actually writes, and it supports the
//! snapshots a later milestone will want.
//!
//! Following ADR 0003, the argument vector is built by a pure function that the
//! tests can assert on, separately from the code that runs the process.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use crate::preflight::QEMU_IMG_BINARY;
use crate::{Error, Result};

/// `qemu-img create -f qcow2 <image> <size>G`
pub fn create_args(image: &Path, size_gib: u64) -> Vec<OsString> {
    vec![
        OsString::from("create"),
        OsString::from("-f"),
        OsString::from("qcow2"),
        image.as_os_str().to_owned(),
        OsString::from(format!("{size_gib}G")),
    ]
}

/// Create a new qcow2 image.
///
/// `qemu-img create` truncates an existing file without asking, which for a
/// disk image means destroying a Windows installation. The check here is what
/// stands between a mistyped VM name and someone's guest.
pub fn create(qemu_img: &Path, image: &Path, size_gib: u64) -> Result<()> {
    if image.exists() {
        return Err(Error::DiskExists(image.to_owned()));
    }
    run(qemu_img, &create_args(image, size_gib))
}

/// Run a `qemu-img` subcommand, turning a non-zero exit into an error that
/// carries whatever the tool complained about.
fn run(qemu_img: &Path, args: &[OsString]) -> Result<()> {
    // An argument vector, never a shell string: a path containing spaces,
    // quotes or `$()` is one inert argument (ADR 0003).
    let output = Command::new(qemu_img)
        .args(args)
        .output()
        .map_err(|source| Error::Spawn {
            binary: QEMU_IMG_BINARY.to_owned(),
            source,
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(Error::CommandFailed {
        binary: QEMU_IMG_BINARY.to_owned(),
        status: describe(output.status),
        detail: stderr_detail(&output.stderr),
    })
}

fn describe(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit status {code}"),
        None => "killed by a signal".to_owned(),
    }
}

fn stderr_detail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn builds_a_qcow2_create_command() {
        let args = create_args(Path::new("/vms/win11/disk.qcow2"), 64);
        assert_eq!(
            args,
            vec![
                OsString::from("create"),
                OsString::from("-f"),
                OsString::from("qcow2"),
                OsString::from("/vms/win11/disk.qcow2"),
                OsString::from("64G"),
            ]
        );
    }

    #[test]
    fn a_path_with_shell_metacharacters_stays_one_argument() {
        let image = PathBuf::from("/vms/a b; rm -rf $HOME/disk.qcow2");
        let args = create_args(&image, 1);
        assert_eq!(args.iter().filter(|a| *a == image.as_os_str()).count(), 1);
        assert_eq!(args.len(), 5, "the path must not be split: {args:?}");
    }

    #[test]
    fn refuses_to_overwrite_an_existing_image() {
        let dir = std::env::temp_dir().join("daholyvm-disk-existing");
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("disk.qcow2");
        std::fs::write(&image, b"pretend this is a windows install").unwrap();

        let err = create(Path::new("/nonexistent/qemu-img"), &image, 1).unwrap_err();
        assert!(matches!(err, Error::DiskExists(_)), "{err}");
        // The guard must fire before anything is executed, so the file is intact.
        assert_eq!(
            std::fs::read(&image).unwrap(),
            b"pretend this is a windows install"
        );
    }

    #[test]
    fn a_missing_qemu_img_is_reported_as_a_spawn_failure() {
        let dir = std::env::temp_dir().join("daholyvm-disk-spawn");
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("absent.qcow2");
        let _ = std::fs::remove_file(&image);

        let err = create(Path::new("/nonexistent/qemu-img"), &image, 1).unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }), "{err}");
    }

    #[test]
    fn empty_stderr_does_not_produce_a_dangling_colon() {
        assert_eq!(stderr_detail(b"   \n"), "");
        assert_eq!(stderr_detail(b"could not open\n"), ": could not open");
    }
}
