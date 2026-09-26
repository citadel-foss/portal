//! One event stream per tab. Core publishes without knowing a browser exists; this is the web
//! half of that contract, the mirror of the desktop bridge.

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::Stream;
use portal_core::events::AppEvent;
use tokio::sync::broadcast::error::RecvError;
use std::sync::Arc;

use portal_core::state::AppState;
use tokio_stream::StreamExt;

use crate::auth::StreamGuard;

/// Wraps each domain event as `{ name, payload }` so one `onmessage` handler can route by
/// name, exactly as the desktop bridge routes by Tauri event name.
fn encode(event: &AppEvent) -> Event {
    Event::default().json_data(serde_json::json!({
        "name": event.name(),
        "payload": event.payload(),
    }))
    .unwrap_or_else(|_| Event::default().data("{}"))
}

/// A wallet's events go only to the sessions on that wallet, looked up per event because a
/// session can switch wallets while its stream stays open.
pub fn stream(
    runtime: Arc<AppState>,
    guard: StreamGuard,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut events = runtime.events.subscribe();
    let stream = async_stream::stream! {
        // Moved in so it drops with the stream, which axum does once a write to a closed tab
        // fails — at the latest, the next keepalive.
        let _guard = guard;
        // Told before anything else, so a client knows to fetch its snapshot and start
        // buffering rather than assuming an empty stream means nothing has happened.
        yield Ok(Event::default().event("stream-ready").data("{}"));
        loop {
            match events.recv().await {
                Ok(envelope) => {
                    let mine = envelope.wallet.as_ref().is_none_or(|wallet| {
                        runtime.wallet_of(_guard.session_id()).as_ref() == Some(wallet)
                    });
                    if mine {
                        yield Ok(encode(&envelope.event));
                    }
                }
                // A lagging receiver has missed notifications, not state. Telling it to
                // resynchronize is the only honest answer; silently continuing would leave it
                // certain about entities it never saw change.
                Err(RecvError::Lagged(_)) => {
                    yield Ok(Event::default().event("resync-required").data("{}"))
                }
                Err(RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream.map(|e: Result<Event, Infallible>| e))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
}
