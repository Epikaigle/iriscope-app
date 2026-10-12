//! Real AVI reader and production controls: sleeping, seeking and replacing videos.
#![cfg(target_os = "linux")]
use iriscope_app::{
    test_support::ControllerHarness,
    ui::{AppState, MainWindow},
};
use iriscope_core::{
    capabilities::FrameRate,
    library::{CaptureFileVersion, capture_file_version},
    video::AviMjpegWriter,
};
use slint::ComponentHandle;
use std::{
    cell::Cell,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn pump_until(window: &MainWindow, condition: impl Fn(&MainWindow) -> bool + 'static) {
    let weak = window.as_weak();
    let passed = Rc::new(Cell::new(false));
    let done = Rc::clone(&passed);
    let deadline = Instant::now() + Duration::from_secs(5);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(5),
        move || {
            if let Some(window) = weak.upgrade()
                && condition(&window)
            {
                done.set(true);
                slint::quit_event_loop().unwrap();
            } else if Instant::now() >= deadline {
                slint::quit_event_loop().unwrap();
            }
        },
    );
    slint::run_event_loop_until_quit().unwrap();
    assert!(passed.get(), "viewer did not reach the expected state");
}

fn pump_for(window: &MainWindow, duration: Duration) {
    let deadline = Instant::now() + duration;
    pump_until(window, move |_| Instant::now() >= deadline);
}

fn pixels(window: &MainWindow) -> Vec<u8> {
    window
        .get_viewer_image()
        .to_rgb8()
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn create_video(path: &Path, fps: FrameRate, blue: bool) -> CaptureFileVersion {
    let mut writer = AviMjpegWriter::create(path, 8, 8, fps).unwrap();
    for frame in 0_u8..4 {
        let color = if blue {
            [0, frame * 30, 220]
        } else {
            [220, frame * 30, 0]
        };
        let rgb: Vec<_> = (0..64).flat_map(|_| color).collect();
        let jpeg = iriscope_imaging::encode_rgb8_jpeg(&rgb, 8, 8, 90).unwrap();
        writer.write_frame(&jpeg).unwrap();
    }
    writer.finish().unwrap();
    drop(writer);
    capture_file_version(path).unwrap()
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[allow(clippy::too_many_lines, clippy::float_cmp)] // Video endpoints and unchanged paused values must be exact.
fn paused_video_seeks_resumes_replaces_and_revalidates_without_polling() {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = Fixture(std::env::temp_dir().join(format!("iriscope-video-playback-{id}")));
    fs::create_dir(&directory.0).unwrap();
    let first = directory.0.join("first.avi");
    let first_version = create_video(&first, FrameRate::new(8, 1).unwrap(), false);
    let cadence = directory.0.join("cadence.avi");
    let cadence_version = create_video(&cadence, FrameRate::new(2, 1).unwrap(), false);
    let slow = directory.0.join("slow.avi");
    let slow_version = create_video(&slow, FrameRate::new(1, 4).unwrap(), true);
    let mut harness = ControllerHarness::new(directory.0.clone()).unwrap();
    harness.start_viewer_worker();
    harness.window().show().unwrap();
    harness.open_viewer_capture(&cadence, &cadence_version);
    pump_until(harness.window(), |window| {
        !window.get_viewer_loading()
            && window.get_viewer_open()
            && window.get_viewer_image().size().width > 0
    });
    let first_visible = Instant::now();
    let first_pixels = pixels(harness.window());
    assert_eq!(harness.window().get_viewer_video_progress(), 0.0);
    pump_for(harness.window(), Duration::from_millis(150));
    assert_eq!(pixels(harness.window()), first_pixels);
    assert_eq!(harness.window().get_viewer_video_progress(), 0.0);
    pump_until(harness.window(), |window| {
        window.get_viewer_video_progress() > 0.0
    });
    assert!(
        first_visible.elapsed() >= Duration::from_millis(350),
        "the first frame observes the half-second AVI cadence"
    );

    harness.open_viewer_capture(&first, &first_version);
    pump_until(harness.window(), |window| {
        !window.get_viewer_loading() && window.get_viewer_video_progress() > 0.0
    });

    harness
        .window()
        .global::<AppState>()
        .invoke_toggle_viewer_video();
    pump_for(harness.window(), Duration::from_millis(100));
    let paused_pixels = pixels(harness.window());
    let paused_progress = harness.window().get_viewer_video_progress();
    pump_for(harness.window(), Duration::from_millis(150));
    assert_eq!(pixels(harness.window()), paused_pixels);
    assert_eq!(
        harness.window().get_viewer_video_progress(),
        paused_progress
    );
    assert!(!harness.viewer_observation().playing);

    harness
        .window()
        .global::<AppState>()
        .invoke_seek_viewer_video(1_000.0);
    pump_until(harness.window(), |window| {
        window.get_viewer_video_progress() == 1_000.0
    });
    assert!(
        !harness.viewer_observation().playing,
        "seeking while paused stays paused"
    );
    harness
        .window()
        .global::<AppState>()
        .invoke_toggle_viewer_video();
    pump_until(harness.window(), |window| {
        window.get_viewer_video_progress() < 1_000.0
    });
    harness
        .window()
        .global::<AppState>()
        .invoke_toggle_viewer_video();

    harness.open_viewer_capture(&slow, &slow_version);
    pump_until(harness.window(), |window| {
        !window.get_viewer_loading()
            && window.get_viewer_open()
            && window
                .get_viewer_image()
                .to_rgb8()
                .is_some_and(|rgb| rgb.as_bytes()[2] > 200)
    });
    let started = Instant::now();
    harness
        .window()
        .global::<AppState>()
        .invoke_seek_viewer_video(1_000.0);
    pump_until(harness.window(), |window| {
        window.get_viewer_video_progress() == 1_000.0
    });
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "seek interrupts a four-second frame delay"
    );
    harness
        .window()
        .global::<AppState>()
        .invoke_toggle_viewer_video();
    pump_for(harness.window(), Duration::from_millis(100));
    let generation = harness.viewer_observation().generation;
    harness.close_viewer();
    assert!(!harness.window().get_viewer_open());
    assert!(harness.viewer_observation().generation > generation);

    harness.open_viewer_capture(&slow, &slow_version);
    pump_until(harness.window(), |window| {
        !window.get_viewer_loading() && window.get_viewer_open()
    });
    harness
        .window()
        .global::<AppState>()
        .invoke_toggle_viewer_video();
    pump_for(harness.window(), Duration::from_millis(100));
    let verified_snapshot = pixels(harness.window());
    let mut edited = File::options().write(true).open(&slow).unwrap();
    edited.seek(SeekFrom::Start(2_058)).unwrap();
    edited.write_all(&[0]).unwrap();
    drop(edited);
    pump_for(harness.window(), Duration::from_millis(150));
    assert_eq!(
        pixels(harness.window()),
        verified_snapshot,
        "pause retains the verified snapshot"
    );
    assert!(harness.window().get_viewer_open());
    harness
        .window()
        .global::<AppState>()
        .invoke_toggle_viewer_video();
    pump_until(harness.window(), |window| !window.get_viewer_open());
    assert!(
        !harness.viewer_observation().playing,
        "resume rejects the modified AVI"
    );
    harness.window().hide().unwrap();
}
