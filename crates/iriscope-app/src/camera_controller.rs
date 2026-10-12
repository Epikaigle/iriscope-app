use crate::app_helpers::{
    dispatch_hardware_button, settings_snapshot, stream_configurations_for_quality,
};
use crate::camera_queue::SharedCameraFrame;
use crate::config::NOTICE_ERROR;
use crate::controls_ui::{
    CameraControlRuntimeState, camera_control_key, compatible_saved_camera_control_value,
    default_camera_control_value, merge_camera_control_readbacks, remember_camera_control_value,
    selected_camera_menu_value, set_camera_control_model, snap_integer_control_value,
    update_camera_control_row,
};
use crate::platform_camera;
use crate::playback::{camera_error_status, is_de400, show_capture_notice};
use crate::recording_worker::{RecordingStopReason, stop_recording};
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::camera::{CameraDevice, CameraErrorKind, CameraEvent, CameraResult};
use iriscope_core::capabilities::{
    CameraControlDescriptor, CameraControlId, CameraControlKind, CameraControlValue,
    StandardCameraControl,
};
use iriscope_core::settings::SavedCameraControlValue;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

fn live_preview_is_visible(win: &MainWindow) -> bool {
    win.get_current_tab() == 0
        && !win.get_is_frozen()
        && !win.get_viewer_open()
        && !win.get_library_map_open()
}

#[allow(clippy::too_many_lines)]
pub(super) fn install(main_window: &MainWindow, runtime: &AppRuntime) {
    let latest_frame = Arc::clone(&runtime.latest_frame);
    let decode_mailbox = Arc::clone(&runtime.decode_mailbox);
    let latest_decoded_frame = Arc::clone(&runtime.latest_decoded_frame);
    let preview_active = Arc::clone(&runtime.preview_active);
    let decode_time_micros = Arc::clone(&runtime.decode_time_micros);
    let frozen_frame = Arc::clone(&runtime.frozen_frame);
    let camera_controls = Arc::clone(&runtime.camera_controls);
    let control_commands = Arc::clone(&runtime.control_commands);
    // Keep the capture stream running for photos and recordings while avoiding
    // decode work when its preview is hidden.
    let weak_preview = main_window.as_weak();
    let preview_active_tab = Arc::clone(&preview_active);
    let decode_mailbox_tab = Arc::clone(&decode_mailbox);
    let latest_decoded_frame_tab = Arc::clone(&latest_decoded_frame);
    let decode_time_micros_tab = Arc::clone(&decode_time_micros);
    main_window
        .global::<AppState>()
        .on_preview_tab_changed(move |visible| {
            let active = visible
                && weak_preview
                    .upgrade()
                    .is_some_and(|win| live_preview_is_visible(&win));
            preview_active_tab.store(active, Ordering::Release);
            if !active {
                decode_mailbox_tab.clear();
                if let Ok(mut frame) = latest_decoded_frame_tab.lock() {
                    *frame = None;
                }
                decode_time_micros_tab.store(0, Ordering::Relaxed);
            }
        });

    // Freeze frame
    let weak_freeze = main_window.as_weak();
    let latest_frame_freeze = Arc::clone(&latest_frame);
    let frozen_frame_freeze = Arc::clone(&frozen_frame);
    let preview_active_freeze = Arc::clone(&preview_active);
    let decode_mailbox_freeze = Arc::clone(&decode_mailbox);
    let latest_decoded_frame_freeze = Arc::clone(&latest_decoded_frame);
    let decode_time_micros_freeze = Arc::clone(&decode_time_micros);
    main_window.global::<AppState>().on_toggle_freeze(move || {
        let Some(win) = weak_freeze.upgrade() else {
            return;
        };
        let was_frozen = win.get_is_frozen();
        let new_frozen = !was_frozen;
        win.set_is_frozen(new_frozen);
        preview_active_freeze.store(live_preview_is_visible(&win), Ordering::Release);
        if new_frozen {
            decode_mailbox_freeze.clear();
            if let Ok(mut frame) = latest_decoded_frame_freeze.lock() {
                *frame = None;
            }
            decode_time_micros_freeze.store(0, Ordering::Relaxed);
            if let Ok(mut g) = frozen_frame_freeze.lock() {
                *g = latest_frame_freeze.snapshot();
            }
        } else if let Ok(mut g) = frozen_frame_freeze.lock() {
            *g = None;
        }
    });

    // Camera controls discovered dynamically from the active backend.
    let control_commands_value = Arc::clone(&control_commands);
    let controls_value = Arc::clone(&camera_controls);
    let weak_value = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_set_camera_control_value(move |key, requested| {
            let Some(win) = weak_value.upgrade() else {
                return;
            };
            let Ok(mut controls) = controls_value.lock() else {
                return;
            };
            let Some(index) = controls.iter().position(|state| state.key == key.as_str()) else {
                return;
            };
            let state = &mut controls[index];
            if state.descriptor.read_only {
                return;
            }
            let Some(value) = snap_integer_control_value(&state.descriptor, requested) else {
                return;
            };
            if state.value == value {
                return;
            }
            submit_camera_control_request(state, value, &control_commands_value);
            let updated_controls = controls.clone();
            drop(controls);
            update_camera_control_row(&win, &updated_controls, index);
        });

    let control_commands_step = Arc::clone(&control_commands);
    let controls_step = Arc::clone(&camera_controls);
    let weak_step = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_step_camera_control(move |key, direction| {
            let Some(win) = weak_step.upgrade() else {
                return;
            };
            let Ok(mut controls) = controls_step.lock() else {
                return;
            };
            let Some(index) = controls.iter().position(|state| state.key == key.as_str()) else {
                return;
            };
            let state = &mut controls[index];
            if state.descriptor.read_only {
                return;
            }
            let (
                CameraControlKind::Integer {
                    minimum,
                    maximum,
                    step,
                    ..
                },
                CameraControlValue::Integer(current),
            ) = (&state.descriptor.kind, &state.value)
            else {
                return;
            };
            let next = (i128::from(*current)
                + i128::from(direction.signum()) * i128::from((*step).max(1)))
            .clamp(i128::from(*minimum), i128::from(*maximum));
            let Ok(next) = i64::try_from(next) else {
                return;
            };
            if next == *current {
                return;
            }
            let value = CameraControlValue::Integer(next);
            submit_camera_control_request(state, value, &control_commands_step);
            let updated_controls = controls.clone();
            drop(controls);
            update_camera_control_row(&win, &updated_controls, index);
        });

    let control_commands_bool = Arc::clone(&control_commands);
    let controls_bool = Arc::clone(&camera_controls);
    let weak_bool = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_set_camera_control_bool(move |key, requested| {
            let Some(win) = weak_bool.upgrade() else {
                return;
            };
            let Ok(mut controls) = controls_bool.lock() else {
                return;
            };
            let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
                return;
            };
            if state.descriptor.read_only
                || !matches!(state.descriptor.kind, CameraControlKind::Boolean { .. })
            {
                return;
            }
            let value = CameraControlValue::Boolean(requested);
            submit_camera_control_request(state, value, &control_commands_bool);
            let updated_controls = controls.clone();
            drop(controls);
            set_camera_control_model(&win, &updated_controls);
        });

    let control_commands_menu = Arc::clone(&control_commands);
    let controls_menu = Arc::clone(&camera_controls);
    let weak_menu = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_select_camera_control_menu(move |key, requested_index| {
            let Some(win) = weak_menu.upgrade() else {
                return;
            };
            let Ok(mut controls) = controls_menu.lock() else {
                return;
            };
            let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
                return;
            };
            let Some(value) = selected_camera_menu_value(&state.descriptor, requested_index) else {
                return;
            };
            submit_camera_control_request(state, value, &control_commands_menu);
            let updated_controls = controls.clone();
            drop(controls);
            set_camera_control_model(&win, &updated_controls);
        });

    let control_commands_reset = Arc::clone(&control_commands);
    let controls_reset = Arc::clone(&camera_controls);
    main_window
        .global::<AppState>()
        .on_reset_camera_controls(move || {
            let Ok(mut controls) = controls_reset.lock() else {
                return;
            };
            let revisions = controls
                .iter_mut()
                .map(|state| {
                    state.revision = state.revision.wrapping_add(1);
                    (state.descriptor.id.clone(), state.revision)
                })
                .collect();
            control_commands_reset.reset_controls(revisions);
        });
}

fn submit_camera_control_request(
    state: &mut CameraControlRuntimeState,
    value: CameraControlValue,
    commands: &crate::camera_queue::ControlCommandMailbox,
) {
    state.value.clone_from(&value);
    state.revision = state.revision.wrapping_add(1);
    commands.set_control(state.descriptor.id.clone(), value, state.revision);
}

fn restore_camera_controls(
    device: &mut dyn CameraDevice,
    descriptors: &[CameraControlDescriptor],
    saved_controls: &BTreeMap<String, SavedCameraControlValue>,
) -> HashMap<CameraControlId, CameraControlValue> {
    let mut restore_values = descriptors
        .iter()
        .filter_map(|descriptor| {
            let saved = saved_controls.get(&camera_control_key(&descriptor.id))?;
            Some((
                descriptor,
                compatible_saved_camera_control_value(descriptor, saved)?,
            ))
        })
        .collect::<Vec<_>>();
    let is_mode = |descriptor: &CameraControlDescriptor| {
        matches!(
            descriptor.kind,
            CameraControlKind::Boolean { .. } | CameraControlKind::Menu { .. }
        )
    };
    // Select manual modes before their values (required by some Linux drivers).
    restore_values.sort_by_key(|(descriptor, _)| !is_mode(descriptor));
    let mut applied = HashMap::new();
    for (descriptor, value) in restore_values.iter().chain(
        restore_values
            .iter()
            .filter(|(descriptor, _)| is_mode(descriptor)),
    ) {
        // Windows numeric properties force manual mode. Reapply saved modes
        // last so an automatic mode remains authoritative after reopening.
        if device.set_control_value(&descriptor.id, value).is_ok() {
            applied.insert(descriptor.id.clone(), value.clone());
        }
    }
    applied
}

fn related_camera_control(id: &CameraControlId) -> Option<CameraControlId> {
    let CameraControlId::Standard(control) = id else {
        return None;
    };
    let related = match control {
        StandardCameraControl::Exposure => StandardCameraControl::ExposureMode,
        StandardCameraControl::ExposureMode => StandardCameraControl::Exposure,
        StandardCameraControl::WhiteBalanceManual => StandardCameraControl::WhiteBalanceAutomatic,
        StandardCameraControl::WhiteBalanceAutomatic => StandardCameraControl::WhiteBalanceManual,
        _ => return None,
    };
    Some(CameraControlId::Standard(related))
}

fn apply_camera_control_update(
    device: &mut dyn CameraDevice,
    confirmed: &mut [CameraControlRuntimeState],
    id: &CameraControlId,
    value: &CameraControlValue,
    revision: u64,
) -> (CameraResult<()>, Vec<CameraControlRuntimeState>) {
    let result = device.set_control_value(id, value);
    let related = related_camera_control(id);
    let readbacks = confirmed
        .iter_mut()
        .filter(|state| {
            state.descriptor.id == *id || related.as_ref() == Some(&state.descriptor.id)
        })
        .map(|state| {
            state.value = device
                .control_value(&state.descriptor.id)
                .unwrap_or_else(|_| {
                    if state.descriptor.id == *id && result.is_ok() {
                        value.clone()
                    } else {
                        state.value.clone()
                    }
                });
            if state.descriptor.id == *id {
                state.revision = revision;
            }
            state.clone()
        })
        .collect();
    (result, readbacks)
}

fn read_camera_controls_after_reset(
    device: &dyn CameraDevice,
    confirmed: &mut [CameraControlRuntimeState],
    revisions: &HashMap<CameraControlId, u64>,
    reset_succeeded: bool,
) {
    for state in confirmed {
        state.value = device
            .control_value(&state.descriptor.id)
            .unwrap_or_else(|_| {
                if reset_succeeded {
                    default_camera_control_value(&state.descriptor.kind)
                        .unwrap_or_else(|| state.value.clone())
                } else {
                    state.value.clone()
                }
            });
        if let Some(revision) = revisions.get(&state.descriptor.id) {
            state.revision = *revision;
        }
    }
}

fn publish_camera_control_readbacks(
    window: &slint::Weak<MainWindow>,
    current_controls: &Arc<Mutex<Vec<CameraControlRuntimeState>>>,
    stream_generation: &Arc<AtomicU64>,
    generation: u64,
    readbacks: Vec<CameraControlRuntimeState>,
) {
    let updated = current_controls
        .lock()
        .ok()
        .map(|mut controls| merge_camera_control_readbacks(&mut controls, &readbacks))
        .unwrap_or_default();
    if updated.is_empty() {
        return;
    }
    let ui_generation = Arc::clone(stream_generation);
    let current_controls = Arc::clone(current_controls);
    let _ = window.upgrade_in_event_loop(move |win| {
        if ui_generation.load(Ordering::Acquire) != generation {
            return;
        }
        let Some(controls) = current_controls.lock().ok().map(|states| states.clone()) else {
            return;
        };
        for index in updated {
            if readbacks.iter().any(|readback| {
                controls.get(index).is_some_and(|current| {
                    current.descriptor.id == readback.descriptor.id
                        && current.revision == readback.revision
                        && current.value == readback.value
                })
            }) {
                update_camera_control_row(&win, &controls, index);
            }
        }
    });
}

pub(super) fn start_workers(
    main_window: &MainWindow,
    runtime: &AppRuntime,
) -> Vec<(&'static str, thread::JoinHandle<()>)> {
    vec![
        (
            "décodage caméra",
            start_decoder_worker(main_window, runtime),
        ),
        ("caméra", start_camera_worker(main_window, runtime)),
    ]
}

fn start_decoder_worker(main_window: &MainWindow, runtime: &AppRuntime) -> thread::JoinHandle<()> {
    let decode_mailbox = Arc::clone(&runtime.decode_mailbox);
    let stream_generation = Arc::clone(&runtime.stream_generation);
    let latest_decoded_frame = Arc::clone(&runtime.latest_decoded_frame);
    let decoded_frame_update_pending = Arc::clone(&runtime.decoded_frame_update_pending);
    let preview_active = Arc::clone(&runtime.preview_active);
    let decode_time_micros = Arc::clone(&runtime.decode_time_micros);
    // Decode away from the camera thread. The mailbox contains at most one frame,
    // so a slow decoder always skips ahead instead of increasing display latency.
    let decode_mailbox_worker = Arc::clone(&decode_mailbox);
    let latest_decoded_frame_worker = Arc::clone(&latest_decoded_frame);
    let decoded_frame_update_pending_worker = Arc::clone(&decoded_frame_update_pending);
    let decode_time_micros_worker = Arc::clone(&decode_time_micros);
    let stream_generation_decoder = Arc::clone(&stream_generation);
    let preview_active_decoder = Arc::clone(&preview_active);
    let decode_weak = main_window.as_weak();
    thread::spawn(move || {
        while let Some(job) = decode_mailbox_worker.receive() {
            if !job.is_current(&stream_generation_decoder)
                || !preview_active_decoder.load(Ordering::Acquire)
            {
                continue;
            }
            let decode_start = Instant::now();
            let decoded_frame = job.frame.decode_for_preview();
            let elapsed_micros =
                u64::try_from(decode_start.elapsed().as_micros()).unwrap_or(u64::MAX);
            decode_time_micros_worker.store(elapsed_micros, Ordering::Relaxed);

            let Some(pixels) = decoded_frame else {
                continue;
            };
            if !job.is_current(&stream_generation_decoder)
                || !preview_active_decoder.load(Ordering::Acquire)
            {
                continue;
            }
            if let Ok(mut latest) = latest_decoded_frame_worker.lock() {
                *latest = Some((job.generation, pixels));
            }

            if decoded_frame_update_pending_worker.swap(true, Ordering::AcqRel) {
                continue;
            }

            let latest = Arc::clone(&latest_decoded_frame_worker);
            let pending = Arc::clone(&decoded_frame_update_pending_worker);
            let generation = Arc::clone(&stream_generation_decoder);
            let preview_active = Arc::clone(&preview_active_decoder);
            let update_result = decode_weak.upgrade_in_event_loop(move |win| {
                // Clear the gate before taking the slot. A concurrent publisher can
                // queue one follow-up update, while the queue remains strictly bounded.
                pending.store(false, Ordering::Release);
                let decoded = latest.lock().ok().and_then(|mut slot| slot.take());
                if !preview_active.load(Ordering::Acquire) || !live_preview_is_visible(&win) {
                    return;
                }
                let Some((job_generation, pixel_buffer)) = decoded else {
                    return;
                };
                if job_generation != generation.load(Ordering::Acquire) {
                    return;
                }
                win.set_live_frame(slint::Image::from_rgb8(pixel_buffer));
            });
            if update_result.is_err() {
                decoded_frame_update_pending_worker.store(false, Ordering::Release);
            }
        }
    })
}

// One thread owns the device, control updates, reconnects and recording interruptions.
#[allow(clippy::too_many_lines)]
fn start_camera_worker(main_window: &MainWindow, runtime: &AppRuntime) -> thread::JoinHandle<()> {
    let settings = Arc::clone(&runtime.settings);
    let latest_frame = Arc::clone(&runtime.latest_frame);
    let decode_mailbox = Arc::clone(&runtime.decode_mailbox);
    let stream_generation = Arc::clone(&runtime.stream_generation);
    let stream_restart_requested = Arc::clone(&runtime.stream_restart_requested);
    let latest_decoded_frame = Arc::clone(&runtime.latest_decoded_frame);
    let preview_active = Arc::clone(&runtime.preview_active);
    let decode_time_micros = Arc::clone(&runtime.decode_time_micros);
    let dropped_decode_frames = Arc::clone(&runtime.dropped_decode_frames);
    let active_stream_configuration = Arc::clone(&runtime.active_stream_configuration);
    let recording_mailbox = Arc::clone(&runtime.recording_mailbox);
    let recording_start = Arc::clone(&runtime.recording_start);
    let frozen_frame = Arc::clone(&runtime.frozen_frame);
    let camera_controls = Arc::clone(&runtime.camera_controls);
    let control_commands = Arc::clone(&runtime.control_commands);
    let camera_settings_save = Arc::clone(&runtime.camera_settings_save);
    // Spawn camera capture worker thread
    let main_weak = main_window.as_weak();
    let latest_frame_clone = Arc::clone(&latest_frame);
    let decode_mailbox_camera = Arc::clone(&decode_mailbox);
    let preview_active_camera = Arc::clone(&preview_active);
    let latest_decoded_frame_camera = Arc::clone(&latest_decoded_frame);
    let stream_generation_camera = Arc::clone(&stream_generation);
    let stream_restart_requested_worker = Arc::clone(&stream_restart_requested);
    let decode_time_micros_camera = Arc::clone(&decode_time_micros);
    let dropped_decode_frames_camera = Arc::clone(&dropped_decode_frames);
    let recording_mailbox_camera = Arc::clone(&recording_mailbox);
    let rec_start_clone = Arc::clone(&recording_start);
    let active_stream_configuration_worker = Arc::clone(&active_stream_configuration);
    let camera_controls_worker = Arc::clone(&camera_controls);
    let control_commands_worker = Arc::clone(&control_commands);
    let camera_settings_save_for_camera = Arc::clone(&camera_settings_save);
    let settings_worker = Arc::clone(&settings);
    let frozen_frame_worker = Arc::clone(&frozen_frame);

    thread::spawn(move || {
        let mut backend = platform_camera::create_backend();
        loop {
            if control_commands_worker.is_stopped() {
                return;
            }
            let devices = match backend.enumerate_devices() {
                Ok(devices) => devices,
                Err(error) => {
                    eprintln!("[IrisScope] Caméra indisponible : {error}");
                    let status = format!("{} : {error}", camera_error_status(error.kind()));
                    let _ = main_weak.upgrade_in_event_loop(move |win| {
                        win.set_camera_connected(false);
                        win.global::<AppState>().set_hardware_button_status(
                            crate::app_helpers::hardware_button_status().into(),
                        );
                        win.set_is_streaming(false);
                        win.set_status_text(status.into());
                    });
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            if devices.is_empty() {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.global::<AppState>().set_hardware_button_status(
                        crate::app_helpers::hardware_button_status().into(),
                    );
                    win.set_is_streaming(false);
                    win.set_status_text("DE400 non détecté".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let Some(target) = devices.iter().find(|device| is_de400(device)).cloned() else {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.global::<AppState>().set_hardware_button_status(
                        crate::app_helpers::hardware_button_status().into(),
                    );
                    win.set_is_streaming(false);
                    win.set_status_text("DE400 non détecté".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            };

            let usb_info = target.usb.as_ref().map_or_else(
                || "N/A".to_string(),
                |u| format!("{:04x}:{:04x}", u.vendor_id, u.product_id),
            );

            let mut device = match backend.open(&target.id) {
                Ok(device) => device,
                Err(error) => {
                    eprintln!("[IrisScope] Caméra indisponible : {error}");
                    let status = format!("{} : {error}", camera_error_status(error.kind()));
                    let _ = main_weak.upgrade_in_event_loop(move |win| {
                        win.set_camera_connected(false);
                        win.global::<AppState>().set_hardware_button_status(
                            crate::app_helpers::hardware_button_status().into(),
                        );
                        win.set_is_streaming(false);
                        win.set_status_text(status.into());
                    });
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let candidates = stream_configurations_for_quality(
                device.capabilities(),
                settings_snapshot(&settings_worker).video_quality,
            );

            let mut active_config = None;
            let mut last_start_error = None;
            for candidate in candidates {
                if control_commands_worker.is_stopped() {
                    return;
                }
                match device.start_stream(&candidate) {
                    Ok(()) => {
                        active_config = Some(device.active_configuration().unwrap_or(candidate));
                        break;
                    }
                    Err(error) => last_start_error = Some(error),
                }
            }

            let Some(config) = active_config else {
                let status = last_start_error.map_or_else(
                    || "Flux DE400 indisponible : aucun format compatible".to_owned(),
                    |error| {
                        eprintln!("[IrisScope] Démarrage du flux impossible : {error}");
                        format!("{} : {error}", camera_error_status(error.kind()))
                    },
                );
                let _ = main_weak.upgrade_in_event_loop(move |win| {
                    win.set_camera_connected(false);
                    win.global::<AppState>().set_hardware_button_status(
                        crate::app_helpers::hardware_button_status().into(),
                    );
                    win.set_is_streaming(false);
                    win.set_status_text(status.into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            let descriptors = device.capabilities().controls.clone();
            let saved_controls = settings_snapshot(&settings_worker).camera_control_values;
            let applied_values =
                restore_camera_controls(device.as_mut(), &descriptors, &saved_controls);
            let discovered_controls = descriptors
                .into_iter()
                .filter_map(|descriptor| {
                    let fallback = default_camera_control_value(&descriptor.kind)?;
                    let value = device
                        .control_value(&descriptor.id)
                        .ok()
                        .or_else(|| applied_values.get(&descriptor.id).cloned())
                        .unwrap_or(fallback);
                    Some(CameraControlRuntimeState {
                        key: camera_control_key(&descriptor.id),
                        descriptor,
                        value,
                        revision: 0,
                    })
                })
                .collect::<Vec<_>>();
            let mut confirmed_controls = discovered_controls.clone();
            if let Ok(mut controls) = camera_controls_worker.lock() {
                controls.clone_from(&discovered_controls);
            }

            let generation = stream_generation_camera.fetch_add(1, Ordering::AcqRel) + 1;
            let _ = latest_frame_clone.take();
            decode_mailbox_camera.clear();
            if let Ok(mut latest) = latest_decoded_frame_camera.lock() {
                *latest = None;
            }
            decode_time_micros_camera.store(0, Ordering::Relaxed);
            dropped_decode_frames_camera.store(0, Ordering::Relaxed);
            if let Ok(mut frozen) = frozen_frame_worker.lock() {
                *frozen = None;
            }
            if let Ok(mut active) = active_stream_configuration_worker.lock() {
                *active = Some(config.clone());
            }

            let _ = main_weak.upgrade_in_event_loop({
                let dev_name = target.display_name.clone();
                let dev_id = target.id.to_string();
                let backend_name = backend.kind().to_string();
                let usb_text = usb_info.clone();
                let format_text = config.pixel_format.to_string();
                let res_text = config.resolution.to_string();
                let fps_text = format!("{:.2} fps", config.frame_rate.frames_per_second());
                let controls = discovered_controls.clone();
                let ui_generation = Arc::clone(&stream_generation_camera);
                let preview_active_for_ui = Arc::clone(&preview_active_camera);

                move |win| {
                    if ui_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
                    win.set_camera_connected(true);
                    win.global::<AppState>().set_hardware_button_status(
                        crate::app_helpers::hardware_button_status().into(),
                    );
                    win.set_is_streaming(false);
                    win.set_is_frozen(false);
                    preview_active_for_ui.store(win.get_current_tab() == 0, Ordering::Release);
                    win.set_live_frame(slint::Image::default());
                    win.set_status_text("DE400 connecté — démarrage du flux...".into());

                    let mut diag = win.get_diagnostics();
                    diag.device_name = dev_name.into();
                    diag.device_id = dev_id.into();
                    diag.backend_name = backend_name.into();
                    diag.usb_info = usb_text.into();
                    diag.active_format = format_text.into();
                    diag.active_resolution = res_text.into();
                    diag.active_fps = fps_text.into();
                    win.set_diagnostics(diag);
                    set_camera_control_model(&win, &controls);
                }
            });

            // Streaming loop
            let mut frame_count: u64 = 0;
            let mut last_stat_time = Instant::now();
            let mut stat_frames = 0;
            let mut stream_ready_announced = false;
            let mut last_frame_at = Instant::now();

            loop {
                if control_commands_worker.is_stopped() {
                    stop_recording(
                        &recording_mailbox_camera,
                        &rec_start_clone,
                        RecordingStopReason::Interrupted,
                    );
                    let _ = device.stop_stream();
                    return;
                }
                if stream_restart_requested_worker.swap(false, Ordering::AcqRel) {
                    stop_recording(
                        &recording_mailbox_camera,
                        &rec_start_clone,
                        RecordingStopReason::Interrupted,
                    );
                    let _ = device.stop_stream();
                    stream_generation_camera.fetch_add(1, Ordering::AcqRel);
                    decode_mailbox_camera.clear();
                    if let Ok(mut decoded) = latest_decoded_frame_camera.lock() {
                        *decoded = None;
                    }
                    if let Ok(mut active) = active_stream_configuration_worker.lock() {
                        *active = None;
                    }
                    let _ = main_weak.upgrade_in_event_loop(|win| {
                        win.set_is_streaming(false);
                        win.set_status_text("Changement de qualité vidéo…".into());
                    });
                    break;
                }
                // Multiple slider changes to the same control collapse to the
                // newest value, while shutdown remains highest priority.
                let commands = control_commands_worker.take();
                if commands.stop {
                    stop_recording(
                        &recording_mailbox_camera,
                        &rec_start_clone,
                        RecordingStopReason::Interrupted,
                    );
                    let _ = device.stop_stream();
                    return;
                }
                if let Some(revisions) = commands.reset {
                    let result = device.reset_controls();
                    read_camera_controls_after_reset(
                        device.as_ref(),
                        &mut confirmed_controls,
                        &revisions,
                        result.is_ok(),
                    );
                    publish_camera_control_readbacks(
                        &main_weak,
                        &camera_controls_worker,
                        &stream_generation_camera,
                        generation,
                        confirmed_controls.clone(),
                    );
                    match result {
                        Ok(()) => {
                            if let Ok(mut guard) = settings_worker.lock() {
                                guard.camera_control_values.clear();
                                camera_settings_save_for_camera.mark_dirty();
                            }
                        }
                        Err(error) => {
                            let message = format!(
                                "Réinitialisation des réglages caméra impossible : {error}"
                            );
                            let _ = main_weak.upgrade_in_event_loop(move |win| {
                                show_capture_notice(&win, message, NOTICE_ERROR);
                            });
                        }
                    }
                }
                for (id, value, revision) in commands.updates {
                    if !confirmed_controls
                        .iter()
                        .any(|state| state.descriptor.id == id)
                    {
                        continue;
                    }
                    let (result, readbacks) = apply_camera_control_update(
                        device.as_mut(),
                        &mut confirmed_controls,
                        &id,
                        &value,
                        revision,
                    );
                    if result.is_ok() {
                        for state in &readbacks {
                            remember_camera_control_value(
                                &settings_worker,
                                &camera_settings_save_for_camera,
                                &state.key,
                                &state.value,
                            );
                        }
                    }
                    publish_camera_control_readbacks(
                        &main_weak,
                        &camera_controls_worker,
                        &stream_generation_camera,
                        generation,
                        readbacks,
                    );
                    if let Err(error) = result {
                        let message = format!("Réglage caméra refusé : {error}");
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            show_capture_notice(&win, message, NOTICE_ERROR);
                        });
                    }
                }

                match device.next_event(Duration::from_millis(500)) {
                    Ok(CameraEvent::Frame(frame)) => {
                        last_frame_at = Instant::now();
                        frame_count = frame_count.saturating_add(1);
                        stat_frames += 1;

                        latest_frame_clone.publish(frame.clone());
                        let shared_frame = Arc::new(SharedCameraFrame::new(frame));
                        if preview_active_camera.load(Ordering::Acquire)
                            && decode_mailbox_camera
                                .publish_shared(generation, Arc::clone(&shared_frame))
                        {
                            dropped_decode_frames_camera.fetch_add(1, Ordering::Relaxed);
                        }
                        recording_mailbox_camera.publish_shared_frame(shared_frame);

                        if !stream_ready_announced {
                            stream_ready_announced = true;
                            let _ = main_weak.upgrade_in_event_loop(|win| {
                                win.set_is_streaming(true);
                                win.set_status_text("DE400 Connecté".into());
                            });
                        }

                        // Update FPS & Timer metrics every 1s
                        if last_stat_time.elapsed() >= Duration::from_secs(1) {
                            let fps =
                                f64::from(stat_frames) / last_stat_time.elapsed().as_secs_f64();
                            let decode_ms = Duration::from_micros(
                                decode_time_micros_camera.load(Ordering::Relaxed),
                            )
                            .as_secs_f64()
                                * 1_000.0;
                            let dropped_frames =
                                dropped_decode_frames_camera.load(Ordering::Relaxed);
                            last_stat_time = Instant::now();
                            stat_frames = 0;

                            let button_status = device.hardware_button_status();
                            let rec_dur_str =
                                rec_start_clone.lock().ok().and_then(|g| *g).map(|st| {
                                    let el = st.elapsed().as_secs();
                                    format!("{:02}:{:02}", el / 60, el % 60)
                                });

                            let _ = main_weak.upgrade_in_event_loop(move |win| {
                                if let Some(status) = button_status {
                                    win.global::<AppState>()
                                        .set_hardware_button_status(status.into());
                                }
                                let mut diag = win.get_diagnostics();
                                diag.measured_fps = format!("{fps:.2} fps").into();
                                diag.decode_time_ms = format!("{decode_ms:.1} ms").into();
                                diag.frame_count = i32::try_from(frame_count).unwrap_or(i32::MAX);
                                diag.dropped_frames =
                                    i32::try_from(dropped_frames).unwrap_or(i32::MAX);
                                win.set_diagnostics(diag);

                                if let Some(dur) = rec_dur_str {
                                    win.set_recording_duration(dur.into());
                                }
                            });
                        }
                    }
                    Ok(CameraEvent::HardwareButtonPressed) => {
                        let behavior = settings_snapshot(&settings_worker).physical_button_behavior;
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            dispatch_hardware_button(&win, behavior);
                        });
                    }
                    Ok(CameraEvent::Disconnected) => {
                        let disconnected_generation =
                            stream_generation_camera.fetch_add(1, Ordering::AcqRel) + 1;
                        decode_mailbox_camera.clear();
                        if let Ok(mut latest) = latest_decoded_frame_camera.lock() {
                            *latest = None;
                        }
                        stop_recording(
                            &recording_mailbox_camera,
                            &rec_start_clone,
                            RecordingStopReason::Disconnected,
                        );
                        let _ = device.stop_stream();
                        let _ = latest_frame_clone.take();
                        if let Ok(mut frozen) = frozen_frame_worker.lock() {
                            *frozen = None;
                        }
                        if let Ok(mut active) = active_stream_configuration_worker.lock() {
                            *active = None;
                        }
                        if let Ok(mut controls) = camera_controls_worker.lock() {
                            controls.clear();
                        }
                        let ui_generation = Arc::clone(&stream_generation_camera);
                        let recording_mailbox_for_ui = Arc::clone(&recording_mailbox_camera);
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            if ui_generation.load(Ordering::Acquire) != disconnected_generation {
                                return;
                            }
                            win.set_camera_connected(false);
                            win.global::<AppState>().set_hardware_button_status(
                                crate::app_helpers::hardware_button_status().into(),
                            );
                            win.set_is_streaming(false);
                            win.set_is_recording(false);
                            win.set_recording_finalizing(recording_mailbox_for_ui.is_finalizing());
                            win.set_is_frozen(false);
                            win.set_live_frame(slint::Image::default());
                            win.set_recording_duration("00:00".into());
                            win.set_status_text(
                                "DE400 déconnecté — reconnexion en cours...".into(),
                            );
                            win.set_camera_controls(ModelRc::new(VecModel::from(Vec::new())));
                        });
                        break;
                    }
                    Err(error)
                        if error.kind() == CameraErrorKind::TimedOut
                            && last_frame_at.elapsed() < Duration::from_secs(3) => {}
                    Err(error) => {
                        eprintln!("Camera stream interrupted: {error}");
                        let interrupted_generation =
                            stream_generation_camera.fetch_add(1, Ordering::AcqRel) + 1;
                        decode_mailbox_camera.clear();
                        if let Ok(mut latest) = latest_decoded_frame_camera.lock() {
                            *latest = None;
                        }
                        let error_kind = error.kind();
                        stop_recording(
                            &recording_mailbox_camera,
                            &rec_start_clone,
                            RecordingStopReason::Interrupted,
                        );
                        let _ = device.stop_stream();
                        let _ = latest_frame_clone.take();
                        if let Ok(mut frozen) = frozen_frame_worker.lock() {
                            *frozen = None;
                        }
                        if let Ok(mut active) = active_stream_configuration_worker.lock() {
                            *active = None;
                        }
                        if let Ok(mut controls) = camera_controls_worker.lock() {
                            controls.clear();
                        }
                        let status = if matches!(
                            error_kind,
                            CameraErrorKind::Disconnected | CameraErrorKind::DeviceNotFound
                        ) {
                            "DE400 déconnecté — reconnexion en cours...".to_owned()
                        } else {
                            format!(
                                "{} — reconnexion en cours...",
                                camera_error_status(error_kind)
                            )
                        };
                        let ui_generation = Arc::clone(&stream_generation_camera);
                        let recording_mailbox_for_ui = Arc::clone(&recording_mailbox_camera);
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            if ui_generation.load(Ordering::Acquire) != interrupted_generation {
                                return;
                            }
                            win.set_camera_connected(false);
                            win.global::<AppState>().set_hardware_button_status(
                                crate::app_helpers::hardware_button_status().into(),
                            );
                            win.set_is_streaming(false);
                            win.set_is_recording(false);
                            win.set_recording_finalizing(recording_mailbox_for_ui.is_finalizing());
                            win.set_is_frozen(false);
                            win.set_live_frame(slint::Image::default());
                            win.set_recording_duration("00:00".into());
                            win.set_status_text(status.into());
                            win.set_camera_controls(ModelRc::new(VecModel::from(Vec::new())));
                        });
                        thread::sleep(Duration::from_millis(100));
                        break;
                    }
                    Ok(_) => {}
                }
            }
        }
    })
}

#[cfg(test)]
mod control_restoration_tests {
    use super::*;
    use iriscope_core::camera::{
        CameraBackendKind, CameraDescriptor, CameraDeviceId, CameraError, StreamConfiguration,
    };
    use iriscope_core::capabilities::{CameraCapabilities, CameraControlMenuItem};

    struct ModeSwitchingCamera {
        descriptor: CameraDescriptor,
        capabilities: CameraCapabilities,
        values: HashMap<CameraControlId, CameraControlValue>,
        requires_manual: bool,
    }

    fn id(control: StandardCameraControl) -> CameraControlId {
        CameraControlId::Standard(control)
    }

    impl ModeSwitchingCamera {
        fn new(requires_manual: bool) -> Self {
            let controls = [
                (
                    StandardCameraControl::WhiteBalanceAutomatic,
                    CameraControlKind::Boolean { default: true },
                ),
                (
                    StandardCameraControl::WhiteBalanceManual,
                    CameraControlKind::Integer {
                        minimum: 0,
                        maximum: 100,
                        step: 1,
                        default: 10,
                        unit: None,
                    },
                ),
                (
                    StandardCameraControl::ExposureMode,
                    CameraControlKind::Menu {
                        items: vec![
                            CameraControlMenuItem {
                                value: 1,
                                label: "Automatic".to_owned(),
                            },
                            CameraControlMenuItem {
                                value: 2,
                                label: "Manual".to_owned(),
                            },
                        ],
                        default: 1,
                    },
                ),
                (
                    StandardCameraControl::Exposure,
                    CameraControlKind::Integer {
                        minimum: 0,
                        maximum: 100,
                        step: 1,
                        default: 10,
                        unit: None,
                    },
                ),
            ]
            .into_iter()
            .map(|(control, kind)| CameraControlDescriptor {
                id: id(control),
                name: format!("{control:?}"),
                kind,
                read_only: false,
            })
            .collect::<Vec<_>>();
            let values = controls
                .iter()
                .map(|descriptor| {
                    (
                        descriptor.id.clone(),
                        default_camera_control_value(&descriptor.kind).expect("default"),
                    )
                })
                .collect();
            Self {
                descriptor: CameraDescriptor {
                    id: CameraDeviceId::new("mock"),
                    display_name: "Mock".to_owned(),
                    backend: CameraBackendKind::MediaFoundation,
                    usb: None,
                },
                capabilities: CameraCapabilities {
                    controls,
                    modes: Vec::new(),
                },
                values,
                requires_manual,
            }
        }

        fn states(&self) -> Vec<CameraControlRuntimeState> {
            self.capabilities
                .controls
                .iter()
                .map(|descriptor| CameraControlRuntimeState {
                    key: camera_control_key(&descriptor.id),
                    descriptor: descriptor.clone(),
                    value: self.values[&descriptor.id].clone(),
                    revision: 0,
                })
                .collect()
        }
    }

    impl CameraDevice for ModeSwitchingCamera {
        fn descriptor(&self) -> &CameraDescriptor {
            &self.descriptor
        }
        fn capabilities(&self) -> &CameraCapabilities {
            &self.capabilities
        }
        fn start_stream(&mut self, _: &StreamConfiguration) -> CameraResult<()> {
            Ok(())
        }
        fn stop_stream(&mut self) -> CameraResult<()> {
            Ok(())
        }
        fn next_event(&mut self, _: Duration) -> CameraResult<CameraEvent> {
            Err(CameraError::new(CameraErrorKind::TimedOut, "no frames"))
        }
        fn control_value(&self, control_id: &CameraControlId) -> CameraResult<CameraControlValue> {
            self.values
                .get(control_id)
                .cloned()
                .ok_or_else(|| CameraError::new(CameraErrorKind::Unsupported, "unknown control"))
        }
        fn set_control_value(
            &mut self,
            control_id: &CameraControlId,
            value: &CameraControlValue,
        ) -> CameraResult<()> {
            let manual_mode = match control_id {
                CameraControlId::Standard(StandardCameraControl::WhiteBalanceManual) => Some((
                    id(StandardCameraControl::WhiteBalanceAutomatic),
                    CameraControlValue::Boolean(false),
                )),
                CameraControlId::Standard(StandardCameraControl::Exposure) => Some((
                    id(StandardCameraControl::ExposureMode),
                    CameraControlValue::Menu(2),
                )),
                _ => None,
            };
            if let Some((mode, manual)) = manual_mode {
                if self.requires_manual && self.values.get(&mode) != Some(&manual) {
                    return Err(CameraError::new(
                        CameraErrorKind::Unsupported,
                        "manual mode required",
                    ));
                }
                // Like Windows, setting the numeric value changes its shared mode.
                self.values.insert(mode, manual);
            }
            self.values.insert(control_id.clone(), value.clone());
            Ok(())
        }
        fn reset_controls(&mut self) -> CameraResult<()> {
            Ok(())
        }
    }

    fn saved_controls(automatic: bool) -> BTreeMap<String, SavedCameraControlValue> {
        [
            (
                StandardCameraControl::WhiteBalanceAutomatic,
                SavedCameraControlValue::Boolean(automatic),
            ),
            (
                StandardCameraControl::WhiteBalanceManual,
                SavedCameraControlValue::Integer(42),
            ),
            (
                StandardCameraControl::ExposureMode,
                SavedCameraControlValue::Menu(if automatic { 1 } else { 2 }),
            ),
            (
                StandardCameraControl::Exposure,
                SavedCameraControlValue::Integer(71),
            ),
        ]
        .into_iter()
        .map(|(control, value)| (camera_control_key(&id(control)), value))
        .collect()
    }

    #[test]
    fn restored_automatic_modes_survive_windows_numeric_properties() {
        let mut device = ModeSwitchingCamera::new(false);
        let descriptors = device.capabilities.controls.clone();
        restore_camera_controls(&mut device, &descriptors, &saved_controls(true));
        assert_eq!(
            device.values[&id(StandardCameraControl::Exposure)],
            CameraControlValue::Integer(71)
        );
        assert_eq!(
            device.values[&id(StandardCameraControl::WhiteBalanceManual)],
            CameraControlValue::Integer(42)
        );
        assert_eq!(
            device.values[&id(StandardCameraControl::ExposureMode)],
            CameraControlValue::Menu(1)
        );
        assert_eq!(
            device.values[&id(StandardCameraControl::WhiteBalanceAutomatic)],
            CameraControlValue::Boolean(true)
        );
    }

    #[test]
    fn restored_manual_modes_precede_linux_numeric_properties() {
        let mut device = ModeSwitchingCamera::new(true);
        let descriptors = device.capabilities.controls.clone();
        restore_camera_controls(&mut device, &descriptors, &saved_controls(false));
        assert_eq!(
            device.values[&id(StandardCameraControl::Exposure)],
            CameraControlValue::Integer(71)
        );
        assert_eq!(
            device.values[&id(StandardCameraControl::WhiteBalanceManual)],
            CameraControlValue::Integer(42)
        );
        assert_eq!(
            device.values[&id(StandardCameraControl::ExposureMode)],
            CameraControlValue::Menu(2)
        );
        assert_eq!(
            device.values[&id(StandardCameraControl::WhiteBalanceAutomatic)],
            CameraControlValue::Boolean(false)
        );
    }

    #[test]
    fn numeric_readback_refreshes_automatic_mode_and_preserves_newer_requests() {
        let mut device = ModeSwitchingCamera::new(false);
        let mut confirmed = device.states();
        let mut desired = confirmed.clone();
        let white_balance = id(StandardCameraControl::WhiteBalanceManual);
        let automatic = id(StandardCameraControl::WhiteBalanceAutomatic);
        let numeric_index = desired
            .iter()
            .position(|state| state.descriptor.id == white_balance)
            .expect("numeric");
        let mode_index = desired
            .iter()
            .position(|state| state.descriptor.id == automatic)
            .expect("mode");
        desired[numeric_index].value = CameraControlValue::Integer(42);
        desired[numeric_index].revision = 1;
        let (result, readbacks) = apply_camera_control_update(
            &mut device,
            &mut confirmed,
            &white_balance,
            &CameraControlValue::Integer(42),
            1,
        );
        result.expect("numeric update");
        let mut current = desired.clone();
        merge_camera_control_readbacks(&mut current, &readbacks);
        assert_eq!(
            current[mode_index].value,
            CameraControlValue::Boolean(false)
        );

        // A new Auto request (even the same value as before) must win over this readback.
        desired[mode_index].revision = 1;
        desired[numeric_index].revision = 2;
        desired[numeric_index].value = CameraControlValue::Integer(71);
        assert!(merge_camera_control_readbacks(&mut desired, &readbacks).is_empty());
        assert_eq!(desired[mode_index].value, CameraControlValue::Boolean(true));
        assert_eq!(
            desired[numeric_index].value,
            CameraControlValue::Integer(71)
        );
        let (result, readbacks) = apply_camera_control_update(
            &mut device,
            &mut confirmed,
            &automatic,
            &CameraControlValue::Boolean(true),
            1,
        );
        result.expect("automatic update");
        merge_camera_control_readbacks(&mut desired, &readbacks);
        assert_eq!(desired[mode_index].value, CameraControlValue::Boolean(true));
        assert_eq!(
            desired[numeric_index].value,
            CameraControlValue::Integer(71)
        );
    }

    #[test]
    fn selecting_the_current_automatic_mode_after_a_numeric_request_keeps_auto() {
        for (numeric, automatic, auto_value) in [
            (
                StandardCameraControl::WhiteBalanceManual,
                StandardCameraControl::WhiteBalanceAutomatic,
                CameraControlValue::Boolean(true),
            ),
            (
                StandardCameraControl::Exposure,
                StandardCameraControl::ExposureMode,
                CameraControlValue::Menu(1),
            ),
        ] {
            let mut device = ModeSwitchingCamera::new(false);
            let mut confirmed = device.states();
            let mut desired = confirmed.clone();
            let commands = crate::camera_queue::ControlCommandMailbox::default();
            let numeric_index = desired
                .iter()
                .position(|state| state.descriptor.id == id(numeric))
                .expect("numeric");
            let mode_index = desired
                .iter()
                .position(|state| state.descriptor.id == id(automatic))
                .expect("mode");
            submit_camera_control_request(
                &mut desired[numeric_index],
                CameraControlValue::Integer(71),
                &commands,
            );
            // Popup selection is explicit even when its displayed value is already Auto.
            assert_eq!(desired[mode_index].value, auto_value);
            submit_camera_control_request(&mut desired[mode_index], auto_value.clone(), &commands);
            let batch = commands.take();
            assert_eq!(batch.updates.len(), 2);
            for (control, value, revision) in batch.updates {
                let (result, readbacks) = apply_camera_control_update(
                    &mut device,
                    &mut confirmed,
                    &control,
                    &value,
                    revision,
                );
                result.expect("queued update");
                merge_camera_control_readbacks(&mut desired, &readbacks);
            }
            assert_eq!(device.values[&id(automatic)], auto_value);
            assert_eq!(desired[mode_index].value, auto_value);
            assert_eq!(desired[mode_index].revision, 1);
            assert_eq!(device.values[&id(numeric)], CameraControlValue::Integer(71));
        }
    }

    #[test]
    fn reset_readback_replaces_cancelled_requests_and_preserves_newer_slider_revision() {
        let mut device = ModeSwitchingCamera::new(false);
        let mut confirmed = device.states();
        let mut desired = confirmed.clone();
        let control = id(StandardCameraControl::Exposure);
        let index = desired
            .iter()
            .position(|state| state.descriptor.id == control)
            .expect("exposure");
        desired[index].value = CameraControlValue::Integer(71);
        // Revision 1 was cancelled by reset, which advances the UI request to 2.
        desired[index].revision = 2;
        let revisions = HashMap::from([(control.clone(), 2)]);
        read_camera_controls_after_reset(&device, &mut confirmed, &revisions, true);
        merge_camera_control_readbacks(&mut desired, &confirmed);
        assert_eq!(desired[index].revision, 2);
        assert_eq!(desired[index].value, CameraControlValue::Integer(10));

        // A later slider request must survive even a delayed reset callback.
        desired[index].revision = 3;
        desired[index].value = CameraControlValue::Integer(71);
        merge_camera_control_readbacks(&mut desired, &confirmed);
        assert_eq!(desired[index].revision, 3);
        assert_eq!(desired[index].value, CameraControlValue::Integer(71));
        let (result, readbacks) = apply_camera_control_update(
            &mut device,
            &mut confirmed,
            &control,
            &CameraControlValue::Integer(71),
            3,
        );
        result.expect("slider after reset");
        assert!(merge_camera_control_readbacks(&mut desired, &readbacks).contains(&index));
        assert_eq!(desired[index].value, CameraControlValue::Integer(71));
    }
}
