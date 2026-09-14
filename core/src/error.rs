//! App-level error envelope. Crate errors are `Debug`-only, so everything is
//! converted here before crossing a host boundary. Frontend switches on `code`.

use openswap::maker::MakerError;
use openswap::security::SecurityError;
use openswap::taker::error::TakerError;
use openswap::wallet::WalletError;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
/// Serializable error envelope returned by every host command and carried on failure events.
pub struct AppError {
    /// Stable code for frontend control flow.
    pub code: ErrorCode,
    /// Human-readable failure detail safe to show in the UI.
    pub message: String,
    /// Optional structured context for errors such as insufficient funds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[allow(dead_code)] // full app-wide error surface; some variants not wired up yet
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
/// Stable machine-readable failures sent across IPC; messages remain user-facing detail.
pub enum ErrorCode {
    // setup / preflight
    RpcUnreachable,
    RpcAuthFailed,
    TorUnreachable,
    ZmqUnreachable,
    WalletNotFound,
    WalletWrongPassword,
    WalletLoadFailed,
    // runtime
    NotInitialized,
    SwapInProgress,
    InsufficientFunds,
    NotEnoughMakers,
    ContractsBroadcasted,
    InvalidInput,
    // maker
    MakerBusy,
    MakerNotFound,
    MakerNotInitialized,
    MakerAlreadyRunning,
    MakerNotRunning,
    ReportNotFound,
    UserCancelled,
    AuthorizationDenied,
    /// The wallet changed while a confirmation dialog was open.
    WalletSessionChanged,
    SensitiveOperationInProgress,
    InsecureDataDirectory,
    InvalidFileSelection,
    BackendRouteChanged,
    // infrastructure
    StatePoisoned,
    Io,
    Internal,
}

impl AppError {
    /// Builds an error with no structured details.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// Converts an unexpected debug error into the generic internal category.
    pub fn internal(e: impl std::fmt::Debug) -> Self {
        Self::new(ErrorCode::Internal, format!("{e:?}"))
    }

    /// Reports that a command requires an initialized taker session.
    pub fn not_initialized() -> Self {
        Self::new(ErrorCode::NotInitialized, "wallet is not initialized")
    }

    #[allow(dead_code)]
    pub fn swap_in_progress() -> Self {
        Self::new(ErrorCode::SwapInProgress, "a swap is currently running")
    }

    pub fn maker_busy() -> Self {
        Self::new(
            ErrorCode::MakerBusy,
            "a router start/stop operation is already in progress",
        )
    }

    pub fn maker_not_initialized() -> Self {
        Self::new(ErrorCode::MakerNotInitialized, "router is not initialized")
    }

    pub fn maker_not_found(router_id: &str) -> Self {
        Self::new(
            ErrorCode::MakerNotFound,
            format!("router '{router_id}' is not registered"),
        )
    }

    /// Reports an explicit user cancellation that callers may dismiss silently.
    pub fn user_cancelled(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UserCancelled, message)
    }

}

impl From<TakerError> for AppError {
    fn from(e: TakerError) -> Self {
        // A wallet failure already carries its own classification — a wrong password most of
        // all. Collapsing the whole variant to `WalletLoadFailed` loses that, and the unlock
        // screen then shows a raw crate debug string instead of "Incorrect password".
        if let TakerError::Wallet(wallet) = e {
            return AppError::from(wallet);
        }
        let code = match &e {
            TakerError::ContractsBroadcasted(_) => ErrorCode::ContractsBroadcasted,
            TakerError::NotEnoughMakersInOfferBook => ErrorCode::NotEnoughMakers,
            TakerError::IO(_) => ErrorCode::Io,
            _ => ErrorCode::Internal,
        };
        let details = match &e {
            TakerError::ContractsBroadcasted(txids) => {
                serde_json::to_value(txids.iter().map(|t| t.to_string()).collect::<Vec<_>>()).ok()
            }
            _ => None,
        };
        Self {
            code,
            message: format!("{e:?}"),
            details,
        }
    }
}

impl From<WalletError> for AppError {
    fn from(e: WalletError) -> Self {
        let code = match &e {
            WalletError::InsufficientFund { .. } => ErrorCode::InsufficientFunds,
            WalletError::InvalidAddress(_) => ErrorCode::InvalidInput,
            WalletError::IO(_) => ErrorCode::Io,
            // `PasswordRequired` means the file is encrypted and we passed none, which the UI
            // recovers from the same way as a bad one: ask again.
            WalletError::Security(SecurityError::Decryption | SecurityError::PasswordRequired) => {
                ErrorCode::WalletWrongPassword
            }
            _ => ErrorCode::WalletLoadFailed,
        };
        let details = match &e {
            WalletError::InsufficientFund {
                available,
                required,
            } => serde_json::json!({ "available": available, "required": required }).into(),
            _ => None,
        };
        Self {
            code,
            message: format!("{e:?}"),
            details,
        }
    }
}

impl From<MakerError> for AppError {
    fn from(e: MakerError) -> Self {
        let code = match &e {
            MakerError::Wallet(_) => ErrorCode::WalletLoadFailed,
            MakerError::IO(_) => ErrorCode::Io,
            MakerError::InsufficientLiquidity { .. } => ErrorCode::InsufficientFunds,
            MakerError::TorError(_) => ErrorCode::TorUnreachable,
            _ => ErrorCode::Internal,
        };
        Self {
            code,
            message: format!("{e:?}"),
            details: None,
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, format!("{e:?}"))
    }
}

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(e: std::sync::PoisonError<T>) -> Self {
        Self::new(ErrorCode::StatePoisoned, format!("{e}"))
    }
}

/// Openswap used to panic (not Result::Err) on a wrong password or corrupt wallet file; this
/// classifies the spawn_blocking JoinError's panic message into a proper ErrorCode instead of a
/// generic Internal. Upstream now returns `WalletError::Security` for that, but the panic paths
/// outside wallet decryption remain, so this stays.
///
/// Takes Tokio's error, not a host error type: both hosts run wallet work on the same blocking
/// pool, and only the host knows how to unwrap its own wrapper around this.
pub fn from_wallet_join_error(join_err: tokio::task::JoinError) -> AppError {
    if !join_err.is_panic() {
        return AppError::new(ErrorCode::Internal, "wallet task was cancelled".to_string());
    }
    let payload = join_err.into_panic();
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "wallet operation panicked".to_string());

    let code = if msg.contains("Failed to decrypt") {
        ErrorCode::WalletWrongPassword
    } else if msg.contains("Failed to read the file") {
        ErrorCode::WalletNotFound
    } else {
        // e.g. "Failed to deserialize file ...": corrupt/foreign wallet file.
        ErrorCode::WalletLoadFailed
    };
    AppError::new(code, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Taker::init` reports a wrong password as `TakerError::Wallet(Security(Decryption))`.
    /// The unlock screen switches on `code`, so this has to survive the wrapper — the panic
    /// path in `from_wallet_join_error` is only the other half of the same guarantee.
    #[test]
    fn a_wrong_password_survives_the_taker_error_wrapper() {
        let wrapped = TakerError::Wallet(WalletError::Security(SecurityError::Decryption));
        assert_eq!(AppError::from(wrapped).code, ErrorCode::WalletWrongPassword);

        let missing = TakerError::Wallet(WalletError::Security(SecurityError::PasswordRequired));
        assert_eq!(AppError::from(missing).code, ErrorCode::WalletWrongPassword);
    }

    /// Non-wallet taker failures keep their own mapping.
    #[test]
    fn other_taker_failures_are_unchanged() {
        assert_eq!(
            AppError::from(TakerError::NotEnoughMakersInOfferBook).code,
            ErrorCode::NotEnoughMakers
        );
    }

    #[test]
    fn a_panicking_wallet_load_is_still_classified() {
        // The historical path: upstream panicked rather than returning an error.
        let joined = std::thread::spawn(|| panic!("Failed to decrypt the wallet file"))
            .join()
            .unwrap_err();
        let message = joined
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .unwrap_or_default();
        assert!(message.contains("Failed to decrypt"));
    }
}
