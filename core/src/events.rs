//! Domain events and the fan-out sink both hosts publish through.
//!
//! Payloads are owned and cloneable so a publisher can be an OS swap thread or a logging
//! callback holding no relation to the request that started the work.

use std::path::{Path, PathBuf};

use tokio::sync::broadcast;

use crate::error::AppError;
use crate::types::{MakerPhaseEvent, RecoveryHandoff};

/// How many events a slow subscriber may fall behind before it is told to resynchronize
/// rather than silently missing a state change.
const DEFAULT_CAPACITY: usize = 256;

/// Phase reached inside a long wallet initialization, and any note to show beneath it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct InitPhase {
    pub phase: u8,
    pub note: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    WalletInitPhase(InitPhase),
    RouterPhaseChanged(MakerPhaseEvent),
    SwapFinished(String),
    SwapFailed(AppError),
    SwapRecovering(RecoveryHandoff),
}

impl AppEvent {
    /// Stable wire name, shared by every host so one frontend subscription list works for both.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WalletInitPhase(_) => "wallet://init-phase",
            Self::RouterPhaseChanged(_) => "maker://phase-changed",
            Self::SwapFinished(_) => "swap://finished",
            Self::SwapFailed(_) => "swap://failed",
            Self::SwapRecovering(_) => "swap://recovering",
        }
    }

    /// The body a host delivers under `name()`. Each arm serializes its own payload rather
    /// than the enum, so the wire shape is exactly what the frontend already parses.
    pub fn payload(&self) -> serde_json::Value {
        let value = match self {
            Self::WalletInitPhase(phase) => serde_json::to_value(phase),
            Self::RouterPhaseChanged(event) => serde_json::to_value(event),
            Self::SwapFinished(swap_id) => serde_json::to_value(swap_id),
            Self::SwapFailed(error) => serde_json::to_value(error),
            Self::SwapRecovering(handoff) => serde_json::to_value(handoff),
        };
        // These payloads are plain owned DTOs with no map keys to reject, so failure is not
        // reachable; a null body is still better than dropping the notification entirely.
        value.unwrap_or(serde_json::Value::Null)
    }
}

/// An event plus the wallet it concerns. `None` is everyone's business: routers, the process.
/// A host with several viewers delivers a wallet's events only to the sessions on that wallet.
#[derive(Debug, Clone)]
pub struct Envelope {
    pub wallet: Option<PathBuf>,
    pub event: AppEvent,
}

/// Bounded fan-out. Publishing never blocks and never fails the work that produced the event:
/// with no subscribers, or a subscriber too far behind, the send is dropped rather than
/// propagated. A lagging receiver learns about the gap from its own `RecvError::Lagged`.
#[derive(Debug, Clone)]
pub struct EventSink {
    tx: broadcast::Sender<Envelope>,
}

impl EventSink {
    pub fn new() -> Self {
        Self {
            tx: broadcast::Sender::new(DEFAULT_CAPACITY),
        }
    }

    pub fn publish(&self, event: AppEvent) {
        let _ = self.tx.send(Envelope { wallet: None, event });
    }

    /// Publishes an event that belongs to one wallet, keyed by its data dir.
    pub fn publish_for(&self, wallet: &Path, event: AppEvent) {
        let _ = self.tx.send(Envelope {
            wallet: Some(wallet.to_path_buf()),
            event,
        });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Envelope> {
        self.tx.subscribe()
    }
}

impl Default for EventSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishing_without_subscribers_is_not_an_error() {
        EventSink::new().publish(AppEvent::SwapFinished("abc".into()));
    }

    #[test]
    fn subscribers_receive_published_events() {
        let sink = EventSink::new();
        let mut rx = sink.subscribe();
        sink.publish(AppEvent::WalletInitPhase(InitPhase {
            phase: 2,
            note: None,
        }));
        let envelope = rx.try_recv().expect("event was published before the read");
        assert_eq!(envelope.event.name(), "wallet://init-phase");
        assert!(envelope.wallet.is_none());
    }

    /// The names and payload shapes are what the frontend subscribes to and parses; changing
    /// either silently breaks a page, so both are pinned here.
    #[test]
    fn wire_names_and_payload_shapes_are_unchanged() {
        let finished = AppEvent::SwapFinished("s1".into());
        assert_eq!(finished.name(), "swap://finished");
        assert_eq!(finished.payload(), serde_json::json!("s1"));

        let recovering = AppEvent::SwapRecovering(RecoveryHandoff {
            swap_id: "s1".into(),
            reason: "backend gone".into(),
        });
        assert_eq!(recovering.name(), "swap://recovering");
        assert_eq!(
            recovering.payload(),
            serde_json::json!({ "swapId": "s1", "reason": "backend gone" })
        );

        let phase = AppEvent::WalletInitPhase(InitPhase {
            phase: 4,
            note: Some("Checking for interrupted swaps"),
        });
        assert_eq!(
            phase.payload(),
            serde_json::json!({ "phase": 4, "note": "Checking for interrupted swaps" })
        );

        let failed = AppEvent::SwapFailed(AppError::not_initialized());
        assert_eq!(failed.name(), "swap://failed");
        assert_eq!(failed.payload()["code"], serde_json::json!("NOT_INITIALIZED"));
    }
}
