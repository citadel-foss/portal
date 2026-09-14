//! One event stream per tab. Core publishes without knowing a browser exists; this is the web
//! half of that contract, the mirror of the desktop bridge.

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::Stream;
use portal_core::events::AppEvent;
use tokio::sync::broadcast::error::RecvError;
use tokio_stream::StreamExt;

/// Wraps each domain event as `{ name, payload }` so one `onmessage` handler can route by
/// name, exactly as the desktop bridge routes by Tauri event name.
fn encode(event: &AppEvent) -> Event {
    Event::default().json_data(serde_json::json!({
        "name": event.name(),
        "payload": event.payload(),
    }))
    .unwrap_or_else(|_| Event::default().data("{}"))
}

pub fn stream(
    mut events: tokio::sync::broadcast::Receiver<AppEvent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = async_stream::stream! {
        // Told before anything else, so a client knows to fetch its snapshot and start
        // buffering rather than assuming an empty stream means nothing has happened.
        yield Ok(Event::default().event("stream-ready").data("{}"));
        loop {
            match events.recv().await {
                Ok(event) => yield Ok(encode(&event)),
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
