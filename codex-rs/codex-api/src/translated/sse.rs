use crate::common::ResponseEvent;
use crate::error::ApiError;
use codex_client::TransportError;
use eventsource_stream::Event;
use eventsource_stream::EventStreamError;
use futures::Stream;
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

pub(crate) const CLOSED_EARLY: &str =
    "The connection closed before the model finished; nothing further was executed.";

pub(crate) enum Next {
    Event(Event),
    /// The connection closed without a terminal event.
    Closed,
    Failed(ApiError),
    ConsumerGone,
}

pub(crate) async fn next_event<S>(
    stream: &mut S,
    idle_timeout: Duration,
    tx: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
) -> Next
where
    S: Stream<Item = Result<Event, EventStreamError<TransportError>>> + Unpin,
{
    let response = tokio::select! {
        biased;
        _ = tx.closed() => return Next::ConsumerGone,
        response = timeout(idle_timeout, stream.next()) => response,
    };
    match response {
        Ok(Some(Ok(event))) => Next::Event(event),
        Ok(Some(Err(EventStreamError::Transport(error @ TransportError::Policy(_))))) => {
            Next::Failed(ApiError::Transport(error))
        }
        Ok(Some(Err(error))) => Next::Failed(ApiError::Stream(error.to_string())),
        Ok(None) => Next::Closed,
        Err(_) => Next::Failed(ApiError::Stream(
            "The model service stopped responding (idle timeout).".to_string(),
        )),
    }
}
