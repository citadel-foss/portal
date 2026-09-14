//! Boundary validation for values used in paths, secrets, and command configuration.

use std::path::{Component, Path};

use crate::error::{AppError, ErrorCode};

/// Minimum character count accepted for a new wallet password.
pub const MIN_PASSWORD_LENGTH: usize = 8;
/// Maximum UTF-8 byte length accepted for one filesystem leaf name.
pub const MAX_LEAF_NAME_BYTES: usize = 128;

/// Validates a nonblank password against the application length floor.
pub fn validate_password(password: &str, label: &str) -> Result<(), AppError> {
    if password.trim().is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} cannot be empty"),
        ));
    }
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} must be at least {MIN_PASSWORD_LENGTH} characters"),
        ));
    }
    Ok(())
}

/// Validates one portable filesystem leaf, rejecting traversal and reserved device names.
pub fn validate_leaf_name(value: &str, label: &str) -> Result<(), AppError> {
    let path = Path::new(value);
    let invalid_component = path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)));
    let invalid_chars = value.is_empty()
        || value.len() > MAX_LEAF_NAME_BYTES
        || value.chars().any(char::is_control)
        || value.contains(['/', '\\'])
        || value.ends_with(['.', ' ']);
    let stem = value
        .trim_end_matches(['.', ' '])
        .split_once('.')
        .map_or(value, |(stem, _)| stem)
        .to_ascii_uppercase();
    let windows_reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));

    if invalid_component || invalid_chars || windows_reserved {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} must be one safe file name"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_names_reject_traversal_and_platform_devices() {
        for bad in [
            "",
            ".",
            "..",
            "../wallet",
            "a/b",
            "a\\b",
            "CON",
            "LPT1.txt",
            "name.",
        ] {
            assert!(validate_leaf_name(bad, "walletName").is_err(), "{bad}");
        }
        assert!(validate_leaf_name("maker-01_wallet", "walletName").is_ok());
    }

    #[test]
    fn passwords_are_not_trimmed_but_effectively_empty_is_rejected() {
        assert!(validate_password("        ", "password").is_err());
        assert!(validate_password(" pass word ", "password").is_ok());
    }
}
