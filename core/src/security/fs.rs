//! Filesystem helpers that enforce private ownership and permissions for wallet data.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::error::{AppError, ErrorCode};

/// Creates or validates a real directory and restricts it to the current user on Unix.
pub fn ensure_private_dir(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AppError::new(
                ErrorCode::InsecureDataDirectory,
                "data directory must be a real directory, not a symlink",
            ));
        }
    } else {
        fs::create_dir_all(path)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Requires an existing, non-symlink directory owned by and accessible only to this user.
pub fn require_private_dir(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::new(
            ErrorCode::InsecureDataDirectory,
            "selected data directory must be a real directory, not a symlink",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(AppError::new(
                ErrorCode::InsecureDataDirectory,
                "selected data directory must be owned by this user and inaccessible to group/other accounts (0700)",
            ));
        }
    }
    Ok(())
}

/// Atomically opens a regular target for truncating writes and enforces mode 0600 on Unix.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
