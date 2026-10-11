//! Deadline-bound `AVFoundation` control replies without native framework dependencies.

use std::{sync::mpsc::SyncSender, time::Instant};

use iriscope_core::camera::{CameraError, CameraErrorKind, CameraResult};

pub(super) fn dispatch_control_request<T>(
    deadline: Instant,
    response_sender: &SyncSender<CameraResult<T>>,
    operation: impl FnOnce() -> CameraResult<T>,
) {
    // A caller may already have reported a timeout and abandoned its response.
    // Skip queued stale changes before entering AVFoundation; an operation
    // already inside the native driver cannot be cancelled here.
    let result = if Instant::now() < deadline {
        operation()
    } else {
        Err(CameraError::new(
            CameraErrorKind::TimedOut,
            "camera control command expired before the AVFoundation worker could process it",
        ))
    };
    let _ = response_sender.send(result);
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        sync::mpsc,
        time::{Duration, Instant},
    };

    use super::dispatch_control_request;

    #[test]
    fn queued_expired_control_does_not_execute_after_its_caller_abandons_the_reply() {
        let (command_sender, command_receiver) = mpsc::sync_channel(1);
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        command_sender
            .send((Instant::now(), response_sender, 42))
            .expect("queue control request");
        drop(response_receiver);

        let mutations = Cell::new(0);
        let value = Cell::new(7);
        let (deadline, response, requested) = command_receiver.recv().expect("queued request");
        dispatch_control_request(deadline, &response, || {
            mutations.set(mutations.get() + 1);
            value.set(requested);
            Ok(())
        });
        assert_eq!(mutations.get(), 0);
        assert_eq!(value.get(), 7);
    }

    #[test]
    fn queued_current_control_executes_once_and_returns_its_reply() {
        let (command_sender, command_receiver) = mpsc::sync_channel(1);
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        command_sender
            .send((Instant::now() + Duration::from_secs(5), response_sender, 42))
            .expect("queue control request");

        let mutations = Cell::new(0);
        let value = Cell::new(7);
        let (deadline, response, requested) = command_receiver.recv().expect("queued request");
        dispatch_control_request(deadline, &response, || {
            mutations.set(mutations.get() + 1);
            value.set(requested);
            Ok(value.get())
        });
        assert_eq!(
            response_receiver.recv().expect("control reply").unwrap(),
            42
        );
        assert_eq!(mutations.get(), 1);
        assert_eq!(value.get(), 42);
    }
}
