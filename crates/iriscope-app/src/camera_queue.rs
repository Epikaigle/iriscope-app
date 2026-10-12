use crate::app_helpers::{decode_camera_frame_to_rgb8, persist_settings, settings_error};
use crate::config::NOTICE_ERROR;
use crate::playback::show_capture_notice;
use crate::ui::MainWindow;
use iriscope_core::camera::CapturedFrame;
use iriscope_core::capabilities::{CameraControlId, CameraControlValue, PixelFormat};
use iriscope_core::settings::AppSettings;
use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

#[derive(Default)]
pub(super) struct ControlCommandMailbox {
    pub(super) state: Mutex<ControlCommandState>,
}

#[derive(Default)]
pub(super) struct ControlCommandState {
    pub(super) updates: VecDeque<(CameraControlId, CameraControlValue, u64)>,
    pub(super) reset: Option<HashMap<CameraControlId, u64>>,
    pub(super) stop: bool,
}

pub(super) struct ControlCommandBatch {
    pub(super) updates: VecDeque<(CameraControlId, CameraControlValue, u64)>,
    pub(super) reset: Option<HashMap<CameraControlId, u64>>,
    pub(super) stop: bool,
}

impl ControlCommandMailbox {
    pub(super) fn set_control(
        &self,
        id: CameraControlId,
        value: CameraControlValue,
        revision: u64,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.stop {
            if let Some(previous) = state.updates.iter().position(|(key, _, _)| *key == id) {
                state.updates.remove(previous);
            }
            state.updates.push_back((id, value, revision));
        }
    }

    pub(super) fn reset_controls(&self, revisions: HashMap<CameraControlId, u64>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.updates.clear();
        state.reset = Some(revisions);
    }

    pub(super) fn stop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stop = true;
        state.reset = None;
        state.updates.clear();
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop
    }

    pub(super) fn take(&self) -> ControlCommandBatch {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ControlCommandBatch {
            updates: std::mem::take(&mut state.updates),
            reset: std::mem::take(&mut state.reset),
            stop: state.stop,
        }
    }
}

/// Coalesces settings updates before writing them to disk.
#[derive(Default)]
pub(super) struct CameraSettingsSaveMailbox {
    pub(super) state: Mutex<CameraSettingsSaveState>,
    pub(super) changed: Condvar,
}

#[derive(Default)]
pub(super) struct CameraSettingsSaveState {
    pub(super) generation: u64,
    pub(super) dirty: bool,
    pub(super) closed: bool,
}

impl CameraSettingsSaveMailbox {
    pub(super) fn mark_dirty(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
        state.dirty = true;
        self.changed.notify_one();
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        self.changed.notify_one();
    }
}

pub(super) fn run_camera_settings_save_worker(
    mailbox: &CameraSettingsSaveMailbox,
    settings: &Arc<Mutex<AppSettings>>,
    path: &std::path::Path,
    window: Option<&slint::Weak<MainWindow>>,
) {
    let mut state = mailbox
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    loop {
        while !state.dirty && !state.closed {
            state = mailbox
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if !state.closed {
            let generation = state.generation;
            let (next, _) = mailbox
                .changed
                .wait_timeout_while(state, Duration::from_millis(500), |current| {
                    !current.closed && current.generation == generation
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
            if !state.closed && state.generation != generation {
                continue;
            }
        }

        let dirty = std::mem::take(&mut state.dirty);
        let closed = state.closed;
        let saved_generation = state.generation;
        drop(state);
        if dirty {
            let result = persist_settings(settings, path);
            let current_generation = mailbox
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .generation;
            if let Some(window) = window
                && current_generation == saved_generation
            {
                match result {
                    Ok(()) => {
                        let _ = window.upgrade_in_event_loop(|win| {
                            if win.get_settings_feedback().starts_with("Enregistrement") {
                                win.set_settings_feedback("Paramètres enregistrés.".into());
                                win.set_settings_feedback_is_error(false);
                            }
                        });
                    }
                    Err(error) => {
                        eprintln!("Impossible d'enregistrer les paramètres : {error}");
                        let message =
                            format!("Paramètres appliqués, mais non enregistrés : {error}");
                        let _ = window.upgrade_in_event_loop(move |win| {
                            settings_error(&win, &message);
                            show_capture_notice(&win, message, NOTICE_ERROR);
                        });
                    }
                }
            } else if let Err(error) = result {
                eprintln!("Impossible d'enregistrer les paramètres : {error}");
            }
        }
        if closed {
            break;
        }
        state = mailbox
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

#[cfg(test)]
mod control_command_tests {
    use iriscope_core::capabilities::{CameraControlId, CameraControlValue, StandardCameraControl};

    use super::ControlCommandMailbox;

    #[test]
    pub(super) fn slider_burst_keeps_only_the_newest_value_and_stop_wins() {
        let mailbox = ControlCommandMailbox::default();
        let id = CameraControlId::Standard(StandardCameraControl::Brightness);
        for value in 0..10_000 {
            mailbox.set_control(
                id.clone(),
                CameraControlValue::Integer(value),
                u64::try_from(value).expect("positive revision"),
            );
        }
        let batch = mailbox.take();
        assert_eq!(batch.updates.len(), 1);
        assert_eq!(
            batch.updates.front(),
            Some(&(id.clone(), CameraControlValue::Integer(9_999), 9_999))
        );

        mailbox.set_control(id.clone(), CameraControlValue::Integer(1), 10_000);
        mailbox.reset_controls(std::collections::HashMap::from([(id.clone(), 10_001)]));
        mailbox.set_control(id.clone(), CameraControlValue::Integer(2), 10_002);
        let batch = mailbox.take();
        assert_eq!(
            batch
                .reset
                .and_then(|revisions| revisions.get(&id).copied()),
            Some(10_001)
        );
        assert_eq!(
            batch.updates.front(),
            Some(&(id.clone(), CameraControlValue::Integer(2), 10_002))
        );

        mailbox.set_control(id, CameraControlValue::Integer(3), 10_003);
        mailbox.stop();
        let batch = mailbox.take();
        assert!(batch.stop);
        assert!(batch.updates.is_empty());
    }

    #[test]
    pub(super) fn dependent_controls_keep_the_order_of_the_latest_changes() {
        let mailbox = ControlCommandMailbox::default();
        let automatic = CameraControlId::Standard(StandardCameraControl::ExposureMode);
        let manual = CameraControlId::Standard(StandardCameraControl::Exposure);
        mailbox.set_control(manual.clone(), CameraControlValue::Integer(20), 1);
        mailbox.set_control(automatic.clone(), CameraControlValue::Menu(1), 1);
        mailbox.set_control(manual.clone(), CameraControlValue::Integer(30), 2);

        let updates = mailbox.take().updates.into_iter().collect::<Vec<_>>();
        assert_eq!(
            updates,
            vec![
                (automatic, CameraControlValue::Menu(1), 1),
                (manual, CameraControlValue::Integer(30), 2)
            ]
        );
    }
}

#[derive(Debug, Default)]
pub(super) struct DecodeMailbox {
    pub(super) state: Mutex<DecodeMailboxState>,
    pub(super) ready: Condvar,
}

#[derive(Debug, Default)]
pub(super) struct DecodeMailboxState {
    pub(super) frame: Option<QueuedDecodeFrame>,
    pub(super) closed: bool,
}

#[derive(Debug)]
pub(super) struct QueuedDecodeFrame {
    pub(super) generation: u64,
    pub(super) frame: Arc<SharedCameraFrame>,
}

impl QueuedDecodeFrame {
    pub(super) fn is_current(&self, generation: &AtomicU64) -> bool {
        self.generation == generation.load(Ordering::Acquire)
    }
}

impl Drop for QueuedDecodeFrame {
    fn drop(&mut self) {
        // Replaced, hidden, stale and closed preview jobs must not retain RGB
        // images in the recording queue. Only the validation bit remains there.
        let mut preview = self
            .frame
            .preview
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        preview.requested = false;
        preview.pixels = None;
    }
}

type DecodedCameraFrame = SharedPixelBuffer<Rgb8Pixel>;

/// Shares MJPEG validation between recording and the latest-frame preview.
/// Native bytes remain immutable and recording retains at most one RGB image
/// for each outstanding preview job, never one for every queued video frame.
#[derive(Debug)]
pub(super) struct SharedCameraFrame {
    pub(super) frame: CapturedFrame,
    mjpeg_valid: OnceLock<bool>,
    preview: Mutex<PreviewPixels>,
}

#[derive(Debug, Default)]
struct PreviewPixels {
    requested: bool,
    pixels: Option<DecodedCameraFrame>,
}

impl SharedCameraFrame {
    pub(super) fn new(frame: CapturedFrame) -> Self {
        Self {
            frame,
            mjpeg_valid: OnceLock::new(),
            preview: Mutex::new(PreviewPixels::default()),
        }
    }

    pub(super) fn decode_for_preview(&self) -> Option<DecodedCameraFrame> {
        if self.frame.pixel_format != PixelFormat::Mjpeg {
            return decode_camera_frame_to_rgb8(&self.frame).map(|(width, height, rgb)| {
                SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height)
            });
        }
        if !self.validate_mjpeg() {
            return None;
        }
        self.preview
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pixels
            .take()
    }

    pub(super) fn validate_mjpeg(&self) -> bool {
        self.validate_mjpeg_with(|| crate::pixel_buffer::decode_mjpeg_pixels(&self.frame.data).ok())
    }

    fn validate_mjpeg_with(&self, decode: impl FnOnce() -> Option<DecodedCameraFrame>) -> bool {
        *self.mjpeg_valid.get_or_init(|| {
            // OnceLock lets the other worker reuse the result, including failed
            // validation, and permits retry if a decoder unwinds. No camera or
            // mailbox lock is held during decoding or while waiting for it.
            let decoded = decode();
            let valid = decoded.is_some();
            let mut preview = self
                .preview
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if preview.requested {
                preview.pixels = decoded;
            }
            valid
        })
    }
}

impl DecodeMailbox {
    #[cfg(test)]
    pub(super) fn publish(&self, generation: u64, frame: CapturedFrame) -> bool {
        self.publish_shared(generation, Arc::new(SharedCameraFrame::new(frame)))
    }

    pub(super) fn publish_shared(&self, generation: u64, frame: Arc<SharedCameraFrame>) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return false;
        }
        frame
            .preview
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .requested = true;
        let replaced = state
            .frame
            .replace(QueuedDecodeFrame { generation, frame })
            .is_some();
        self.ready.notify_one();
        replaced
    }

    pub(super) fn receive(&self) -> Option<QueuedDecodeFrame> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.frame.is_none() && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.frame.take()
    }

    pub(super) fn clear(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .frame = None;
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.frame = None;
        self.ready.notify_all();
    }
}

#[cfg(test)]
mod shared_camera_tests {
    use super::{DecodeMailbox, SharedCameraFrame};
    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{PixelFormat, Resolution},
    };
    use slint::{Rgb8Pixel, SharedPixelBuffer};
    use std::sync::{Arc, Barrier, atomic::AtomicUsize};
    use std::{sync::atomic::Ordering, time::Duration};

    fn jpeg_frame() -> CapturedFrame {
        CapturedFrame {
            sequence_number: 7,
            timestamp: Duration::from_millis(120),
            pixel_format: PixelFormat::Mjpeg,
            // Native metadata can differ from JPEG dimensions; validation and
            // the preview continue to use the decoder's actual dimensions.
            resolution: Resolution::new(640, 480),
            data: iriscope_imaging::encode_rgb8_jpeg(&[19; 16 * 12 * 3], 16, 12, 95)
                .expect("JPEG fixture")
                .into(),
        }
    }

    #[test]
    fn preview_then_recording_reuses_validation_without_retaining_rgb() {
        let mailbox = DecodeMailbox::default();
        let shared = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(1, Arc::clone(&shared));
        let job = mailbox.receive().unwrap();
        let preview = job.frame.decode_for_preview().unwrap();
        assert_eq!((preview.width(), preview.height()), (16, 12));
        assert!(shared.validate_mjpeg_with(|| panic!("must reuse JPEG validation")));
        assert!(shared.preview.lock().unwrap().pixels.is_none());
        drop(job);
        assert!(!shared.preview.lock().unwrap().requested);
        assert_eq!(shared.frame.sequence_number, 7);
    }

    #[test]
    fn recording_then_preview_reuses_the_same_rgb_allocation() {
        let mailbox = DecodeMailbox::default();
        let shared = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(1, Arc::clone(&shared));
        assert!(shared.validate_mjpeg());
        let cached_pointer = shared
            .preview
            .lock()
            .unwrap()
            .pixels
            .as_ref()
            .unwrap()
            .as_bytes()
            .as_ptr();
        let job = mailbox.receive().unwrap();
        let preview = job.frame.decode_for_preview().unwrap();
        assert_eq!(preview.as_bytes().as_ptr(), cached_pointer);
        assert!(shared.preview.lock().unwrap().pixels.is_none());
    }

    #[test]
    fn concurrent_validation_decodes_only_once() {
        let mailbox = DecodeMailbox::default();
        let shared = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(1, Arc::clone(&shared));
        let calls = AtomicUsize::new(0);
        let start = Barrier::new(2);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    start.wait();
                    assert!(shared.validate_mjpeg_with(|| {
                        calls.fetch_add(1, Ordering::Relaxed);
                        Some(SharedPixelBuffer::<Rgb8Pixel>::new(16, 12))
                    }));
                });
            }
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(
            mailbox
                .receive()
                .unwrap()
                .frame
                .decode_for_preview()
                .is_some()
        );
    }

    #[test]
    fn invalid_jpeg_is_rejected_by_both_workers_and_failure_is_shared() {
        let mailbox = DecodeMailbox::default();
        let mut native = jpeg_frame();
        native.data = Arc::from([0xff, 0xd8, 0xff, 0xd9]);
        let shared = Arc::new(SharedCameraFrame::new(native));
        mailbox.publish_shared(1, Arc::clone(&shared));
        assert!(!shared.validate_mjpeg());
        assert!(
            mailbox
                .receive()
                .unwrap()
                .frame
                .decode_for_preview()
                .is_none()
        );
        assert!(!shared.validate_mjpeg_with(|| panic!("must reuse failed validation")));
        assert!(shared.preview.lock().unwrap().pixels.is_none());
    }

    #[test]
    fn replacement_reconnect_and_close_release_cached_rgb() {
        let mailbox = DecodeMailbox::default();
        let first = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(1, Arc::clone(&first));
        assert!(first.validate_mjpeg());
        assert!(first.preview.lock().unwrap().pixels.is_some());
        let second = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        assert!(mailbox.publish_shared(2, Arc::clone(&second)));
        assert!(first.preview.lock().unwrap().pixels.is_none());
        assert!(first.validate_mjpeg_with(|| panic!("retain validation only")));
        assert!(second.validate_mjpeg());
        let job = mailbox.receive().unwrap();
        assert!(!job.is_current(&std::sync::atomic::AtomicU64::new(3)));
        drop(job);
        assert!(second.preview.lock().unwrap().pixels.is_none());

        let third = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(3, Arc::clone(&third));
        assert!(third.validate_mjpeg());
        mailbox.close();
        assert!(third.preview.lock().unwrap().pixels.is_none());
        assert!(mailbox.receive().is_none());
    }

    #[test]
    fn recording_without_preview_keeps_validation_only() {
        let shared = SharedCameraFrame::new(jpeg_frame());
        assert!(shared.validate_mjpeg());
        assert!(shared.preview.lock().unwrap().pixels.is_none());
        assert!(shared.validate_mjpeg_with(|| panic!("must reuse validation")));
    }

    #[test]
    fn queued_recordings_retain_rgb_only_for_pending_and_in_flight_preview_jobs() {
        let mailbox = DecodeMailbox::default();
        let mut recordings = Vec::new();
        let first = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(1, Arc::clone(&first));
        let in_flight = mailbox.receive().unwrap();
        assert!(first.validate_mjpeg());
        recordings.push(first);

        // Recording may still own old native frames after the preview skipped
        // them. Even a long retained burst must keep only two RGB buffers:
        // the in-flight preview and the latest pending preview.
        for _ in 0..128 {
            let shared = Arc::new(SharedCameraFrame::new(jpeg_frame()));
            mailbox.publish_shared(1, Arc::clone(&shared));
            assert!(shared.validate_mjpeg());
            recordings.push(shared);
            assert_eq!(
                recordings
                    .iter()
                    .filter(|frame| frame.preview.lock().unwrap().pixels.is_some())
                    .count(),
                2
            );
        }

        drop(in_flight);
        mailbox.clear();
        for frame in recordings {
            let preview = frame.preview.lock().unwrap();
            assert!(!preview.requested);
            assert!(preview.pixels.is_none());
            drop(preview);
            assert!(frame.validate_mjpeg_with(|| panic!("only validation must survive")));
        }
    }

    #[test]
    fn replacing_preview_while_recording_decodes_does_not_retain_stale_rgb() {
        let mailbox = DecodeMailbox::default();
        let shared = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(1, Arc::clone(&shared));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();

        std::thread::scope(|scope| {
            let recording_frame = Arc::clone(&shared);
            let worker = scope.spawn(move || {
                recording_frame.validate_mjpeg_with(|| {
                    started_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    Some(SharedPixelBuffer::<Rgb8Pixel>::new(16, 12))
                })
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let replacement = Arc::new(SharedCameraFrame::new(jpeg_frame()));
            let replaced = mailbox.publish_shared(2, replacement);
            finish_tx.send(()).unwrap();
            assert!(worker.join().unwrap());
            assert!(replaced);
        });

        assert!(shared.validate_mjpeg_with(|| panic!("validation stays shared")));
        let preview = shared.preview.lock().unwrap();
        assert!(!preview.requested);
        assert!(preview.pixels.is_none());
    }

    #[test]
    fn closing_preview_while_recording_decodes_discards_rgb_without_losing_validation() {
        let mailbox = DecodeMailbox::default();
        let shared = Arc::new(SharedCameraFrame::new(jpeg_frame()));
        mailbox.publish_shared(1, Arc::clone(&shared));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();

        std::thread::scope(|scope| {
            let recording_frame = Arc::clone(&shared);
            let worker = scope.spawn(move || {
                recording_frame.validate_mjpeg_with(|| {
                    started_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    Some(SharedPixelBuffer::<Rgb8Pixel>::new(16, 12))
                })
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            mailbox.close();
            finish_tx.send(()).unwrap();
            assert!(worker.join().unwrap());
        });

        assert!(mailbox.receive().is_none());
        assert!(shared.validate_mjpeg_with(|| panic!("recording validation survives close")));
        let preview = shared.preview.lock().unwrap();
        assert!(!preview.requested);
        assert!(preview.pixels.is_none());
    }

    #[test]
    fn decoder_unwind_does_not_leave_shared_validation_blocked() {
        let shared = SharedCameraFrame::new(jpeg_frame());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                shared.validate_mjpeg_with(|| panic!("decoder failure"))
            }))
            .is_err()
        );
        assert!(shared.validate_mjpeg());
    }

    #[test]
    fn non_mjpeg_preview_keeps_native_conversion() {
        let mut native = jpeg_frame();
        native.pixel_format = PixelFormat::Bgra8;
        native.resolution = Resolution::new(1, 1);
        native.data = Arc::from([15, 25, 35, 255]);
        let shared = SharedCameraFrame::new(native);
        let pixels = shared.decode_for_preview().unwrap();
        assert_eq!(pixels.as_bytes(), [35, 25, 15]);
        assert!(shared.mjpeg_valid.get().is_none());
    }

    #[test]
    #[ignore = "manual synthetic performance measurement"]
    fn shared_camera_benchmark() {
        use std::{hint::black_box, time::Instant};

        let (width, height) = (1280_u32, 1024_u32);
        let mut state = 0x1234_5678_u32;
        let rgb = (0..width * height * 3)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                state.to_be_bytes()[0]
            })
            .collect::<Vec<_>>();
        let mut native = jpeg_frame();
        native.resolution = Resolution::new(width, height);
        native.data = iriscope_imaging::encode_rgb8_jpeg(&rgb, width, height, 95)
            .unwrap()
            .into();
        drop(rgb);
        let iterations = 40;
        let mut baseline = Vec::new();
        let mut preview_first = Vec::new();
        let mut recording_first = Vec::new();
        for round in 0..3 {
            for mode in [0, 1, 2].into_iter().cycle().skip(round).take(3) {
                let started = Instant::now();
                for _ in 0..iterations {
                    if mode == 0 {
                        let (decoded_width, decoded_height, rgb) =
                            iriscope_imaging::decode_mjpeg_to_rgb8(&native.data).unwrap();
                        black_box(SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                            &rgb,
                            decoded_width,
                            decoded_height,
                        ));
                        black_box(iriscope_imaging::decode_mjpeg_to_rgb8(&native.data).unwrap());
                    } else {
                        let shared = Arc::new(SharedCameraFrame::new(native.clone()));
                        let mailbox = DecodeMailbox::default();
                        mailbox.publish_shared(1, Arc::clone(&shared));
                        let job = mailbox.receive().unwrap();
                        if mode == 1 {
                            black_box(job.frame.decode_for_preview().unwrap());
                            assert!(shared.validate_mjpeg());
                        } else {
                            assert!(shared.validate_mjpeg());
                            black_box(job.frame.decode_for_preview().unwrap());
                        }
                    }
                }
                let millis = started.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations);
                match mode {
                    0 => baseline.push(millis),
                    1 => preview_first.push(millis),
                    _ => recording_first.push(millis),
                }
            }
        }
        eprintln!(
            "MJPEG {width}x{height}, {} bytes, {iterations} frames/round; ms/frame baseline={baseline:?}, shared preview first={preview_first:?}, shared recording first={recording_first:?}",
            native.data.len()
        );
    }
}
