use std::sync::Arc;
use std::time::Duration;

use futures::{Sink, SinkExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

const MAX_QUEUED_FRAMES: usize = 256;
const QUEUED_BYTE_BUDGET: u32 = 16 * 1024 * 1024;
pub(super) const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) enum OutboundMessage {
    // Terminal errors and close frames may drain after auth is revoked.
    Control(Message),
    Notification(Message),
}

pub(super) struct QueuedMessage {
    message: OutboundMessage,
    // Retain the budget until the write completes or is discarded.
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Disconnect {
    Overflow,
    NotificationLag,
    WriterStopped,
}

#[derive(Clone)]
pub(super) struct OutboundQueue {
    tx: mpsc::Sender<QueuedMessage>,
    budget: Arc<Semaphore>,
    large_response: Arc<Semaphore>,
    disconnect: watch::Sender<Option<Disconnect>>,
}

impl OutboundQueue {
    pub fn new() -> (
        Self,
        mpsc::Receiver<QueuedMessage>,
        watch::Receiver<Option<Disconnect>>,
    ) {
        let (tx, rx) = mpsc::channel(MAX_QUEUED_FRAMES);
        let (disconnect, disconnected) = watch::channel(None);
        (
            Self {
                tx,
                budget: Arc::new(Semaphore::new(QUEUED_BYTE_BUDGET as usize)),
                large_response: Arc::new(Semaphore::new(1)),
                disconnect,
            },
            rx,
            disconnected,
        )
    }

    pub fn disconnect_signal(&self) -> watch::Sender<Option<Disconnect>> {
        self.disconnect.clone()
    }

    pub fn abort(&self, reason: Disconnect) {
        stop(&self.disconnect, reason);
    }

    pub fn is_aborted(&self) -> bool {
        self.disconnect.borrow().is_some()
    }

    pub fn send(&self, message: OutboundMessage) -> bool {
        if self.is_aborted() {
            return false;
        }
        let len = match &message {
            OutboundMessage::Control(frame) | OutboundMessage::Notification(frame) => frame.len(),
        };
        let permit = if len <= QUEUED_BYTE_BUDGET as usize {
            self.budget
                .clone()
                .try_acquire_many_owned(len.max(1) as u32)
        } else if matches!(message, OutboundMessage::Control(_)) {
            // Preserve full-snapshot reads alongside small protocol messages.
            self.large_response.clone().try_acquire_owned()
        } else {
            self.abort(Disconnect::Overflow);
            return false;
        };
        if let Ok(permit) = permit
            && self
                .tx
                .try_send(QueuedMessage {
                    message,
                    _permit: permit,
                })
                .is_ok()
        {
            return true;
        }
        // Treat a gap as a disconnect so the client can reconcile from storage.
        self.abort(Disconnect::Overflow);
        false
    }
}

fn stop(signal: &watch::Sender<Option<Disconnect>>, reason: Disconnect) {
    signal.send_if_modified(|current| {
        if current.is_some() {
            return false;
        }
        *current = Some(reason);
        true
    });
}

pub(super) async fn disconnected(signal: &mut watch::Receiver<Option<Disconnect>>) {
    let _ = signal.wait_for(Option::is_some).await;
}

pub(super) async fn write_frames<S>(
    mut sink: S,
    mut queue: mpsc::Receiver<QueuedMessage>,
    disconnect: watch::Sender<Option<Disconnect>>,
    has_execution_authority: impl Fn() -> bool,
    can_send_notifications: impl Fn() -> bool,
) where
    S: Sink<Message> + Unpin,
{
    let mut cancelled = disconnect.subscribe();
    loop {
        let queued = tokio::select! {
            biased;
            _ = disconnected(&mut cancelled) => break,
            queued = queue.recv() => match queued {
                Some(queued) => queued,
                None => break,
            },
        };
        if !has_execution_authority() {
            let _ = tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Close(None))).await;
            break;
        }
        let message = match queued.message {
            OutboundMessage::Control(message) => message,
            OutboundMessage::Notification(message) if can_send_notifications() => message,
            OutboundMessage::Notification(_) => continue,
        };
        let written = tokio::select! {
            biased;
            _ = disconnected(&mut cancelled) => break,
            result = tokio::time::timeout(WRITE_TIMEOUT, sink.send(message)) => result,
        };
        if !matches!(written, Ok(Ok(()))) {
            break;
        }
    }
    stop(&disconnect, Disconnect::WriterStopped);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn full_frame_queue_signals_disconnect_without_waiting_for_reader() {
        let (queue, _receiver, mut disconnect) = OutboundQueue::new();
        for _ in 0..MAX_QUEUED_FRAMES {
            assert!(queue.send(OutboundMessage::Notification(Message::Text("delta".into()))));
        }
        assert!(!queue.send(OutboundMessage::Control(Message::Close(None))));
        assert_eq!(*disconnect.borrow_and_update(), Some(Disconnect::Overflow));
        disconnected(&mut disconnect).await;
        assert!(!queue.send(OutboundMessage::Notification(Message::Text("late".into()))));
    }

    #[tokio::test]
    async fn overflow_cancels_a_stalled_write() {
        let (queue, receiver, _disconnect) = OutboundQueue::new();
        let polled = Arc::new(tokio::sync::Notify::new());
        let sink = futures::sink::unfold(polled.clone(), |polled, _frame| async move {
            polled.notify_one();
            std::future::pending::<Result<Arc<tokio::sync::Notify>, std::io::Error>>().await
        });
        let writer = tokio::spawn(write_frames(
            Box::pin(sink),
            receiver,
            queue.disconnect_signal(),
            || true,
            || true,
        ));
        assert!(queue.send(OutboundMessage::Notification(Message::Text("first".into()))));
        tokio::time::timeout(Duration::from_secs(1), polled.notified())
            .await
            .unwrap();
        for _ in 0..MAX_QUEUED_FRAMES {
            assert!(queue.send(OutboundMessage::Notification(Message::Text("delta".into()))));
        }
        assert!(!queue.send(OutboundMessage::Control(Message::Close(None))));
        tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .expect("overflow should cancel the write immediately")
            .unwrap();
    }

    #[tokio::test]
    async fn frame_in_flight_keeps_byte_budget_until_written() {
        let (queue, mut receiver, disconnect) = OutboundQueue::new();
        let data = Message::Text("x".repeat(QUEUED_BYTE_BUDGET as usize).into());
        assert!(queue.send(OutboundMessage::Control(data)));
        let in_flight = receiver.recv().await.unwrap();
        assert_eq!(queue.budget.available_permits(), 0);
        drop(in_flight);
        assert_eq!(
            queue.budget.available_permits(),
            QUEUED_BYTE_BUDGET as usize
        );
        assert!(queue.send(OutboundMessage::Control(Message::Close(None))));
        assert_eq!(*disconnect.borrow(), None);
    }

    #[tokio::test]
    async fn one_large_response_can_share_the_queue_with_small_frames() {
        let (queue, mut receiver, disconnect) = OutboundQueue::new();
        let large = Message::Text("x".repeat(QUEUED_BYTE_BUDGET as usize + 1).into());
        assert!(queue.send(OutboundMessage::Control(large.clone())));
        let in_flight = receiver.recv().await.unwrap();
        assert!(queue.send(OutboundMessage::Notification(Message::Text("delta".into()))));
        drop(in_flight);
        assert!(queue.send(OutboundMessage::Control(large.clone())));
        assert!(!queue.send(OutboundMessage::Control(large)));
        assert_eq!(*disconnect.borrow(), Some(Disconnect::Overflow));
    }

    #[test]
    fn full_byte_budget_signals_disconnect_before_frame_limit() {
        let (queue, _receiver, disconnect) = OutboundQueue::new();
        let data = Message::Text("x".repeat(QUEUED_BYTE_BUDGET as usize).into());
        assert!(queue.send(OutboundMessage::Notification(data)));
        assert!(!queue.send(OutboundMessage::Control(Message::Text("result".into()))));
        assert_eq!(*disconnect.borrow(), Some(Disconnect::Overflow));
    }
}
