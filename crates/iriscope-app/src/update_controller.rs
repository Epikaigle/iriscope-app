use crate::ui::{AppState, MainWindow};
use iriscope_updater::{InstallTarget, ReadyUpdate, Release};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(test)]
#[path = "../tests/support/software_platform.rs"]
mod software_platform;

pub(super) struct UpdateController {
    target: Option<InstallTarget>,
    ready: Arc<Mutex<Option<ReadyUpdate>>>,
    requested: Rc<Cell<bool>>,
    cancelled: Arc<AtomicBool>,
    worker: Rc<RefCell<Option<std::thread::JoinHandle<()>>>>,
    _startup_timer: slint::Timer,
}

fn install_allowed(window: &MainWindow) -> bool {
    let state = window.global::<AppState>();
    !state.get_recording_busy()
        && !state.get_storage_busy()
        && !state.get_export_busy()
        && !state.get_patient_action_pending()
        && !state.get_file_dialog_pending()
        && !state.get_viewer_review_dirty()
        && !state.get_viewer_review_busy()
        && !state.get_consultation_dirty()
        && !state.get_consultation_saving()
}

fn clear_download<T>(state: &AppState, ready: &Mutex<Option<T>>) {
    ready
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    state.set_update_ready(false);
    state.set_update_progress(0);
}

fn apply_check_state<T>(
    state: &AppState,
    ready: &Mutex<Option<T>>,
    result: Result<Option<&str>, &str>,
) {
    match result {
        Ok(version) => {
            clear_download(state, ready);
            state.set_update_available(version.is_some());
            state.set_update_version(version.unwrap_or_default().into());
            state.set_update_message(version.map_or_else(
                || "Iriscope est à jour.".into(),
                |version| format!("Version {version} disponible.").into(),
            ));
        }
        Err(error) => {
            state.set_update_message(format!("Vérification impossible : {error}").into());
        }
    }
}

impl UpdateController {
    #[allow(clippy::too_many_lines)] // Callback closures share the same controller lifetime.
    pub(super) fn install(window: &MainWindow) -> Self {
        let state = window.global::<AppState>();
        let target = InstallTarget::current();
        let error = target.as_ref().err().map(ToString::to_string);
        let target = target.ok();
        state.set_update_supported(target.is_some());
        state.set_update_message(
            error
                .unwrap_or_else(|| {
                    "Vérification automatique au démarrage. L’installation attend votre clic."
                        .into()
                })
                .into(),
        );
        let available = Arc::new(Mutex::new(None::<Release>));
        let ready = Arc::new(Mutex::new(None::<ReadyUpdate>));
        let cancelled = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let requested = Rc::new(Cell::new(false));
        let worker = Rc::new(RefCell::new(None));

        let weak = window.as_weak();
        let check_target = target.clone();
        let check_busy = Arc::clone(&busy);
        let check_cancelled = Arc::clone(&cancelled);
        let check_available = Arc::clone(&available);
        let check_ready = Arc::clone(&ready);
        let check_worker = Rc::clone(&worker);
        state.on_check_update(move || {
            let Some(target) = check_target.clone() else {
                return;
            };
            if check_busy.swap(true, Ordering::AcqRel) {
                return;
            }
            let Some(window) = weak.upgrade() else {
                check_busy.store(false, Ordering::Release);
                return;
            };
            let state = window.global::<AppState>();
            state.set_update_busy(true);
            state.set_update_progress(0);
            state.set_update_message("Recherche d’une nouvelle version…".into());
            check_cancelled.store(false, Ordering::Release);
            let weak = window.as_weak();
            let busy = Arc::clone(&check_busy);
            let cancelled = Arc::clone(&check_cancelled);
            let available = Arc::clone(&check_available);
            let ready = Arc::clone(&check_ready);
            *check_worker.borrow_mut() = Some(std::thread::spawn(move || {
                let result =
                    iriscope_updater::check(env!("CARGO_PKG_VERSION"), &target, &cancelled);
                let _ = weak.upgrade_in_event_loop(move |window| {
                    busy.store(false, Ordering::Release);
                    let state = window.global::<AppState>();
                    state.set_update_busy(false);
                    match result {
                        Ok(release) => {
                            apply_check_state(
                                &state,
                                &ready,
                                Ok(release.as_ref().map(|release| release.version.as_str())),
                            );
                            *available
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) = release;
                        }
                        Err(error) => {
                            apply_check_state(&state, &ready, Err(&error.to_string()));
                        }
                    }
                });
            }));
        });

        let weak = window.as_weak();
        let download_busy = Arc::clone(&busy);
        let download_cancelled = Arc::clone(&cancelled);
        let download_ready = Arc::clone(&ready);
        let download_worker = Rc::clone(&worker);
        state.on_download_update(move || {
            if download_busy.swap(true, Ordering::AcqRel) {
                return;
            }
            let release = available
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let Some(release) = release else {
                download_busy.store(false, Ordering::Release);
                return;
            };
            let Some(window) = weak.upgrade() else {
                download_busy.store(false, Ordering::Release);
                return;
            };
            let state = window.global::<AppState>();
            clear_download(&state, &download_ready);
            state.set_update_busy(true);
            state.set_update_message("Téléchargement et vérification de la mise à jour…".into());
            download_cancelled.store(false, Ordering::Release);
            let weak = window.as_weak();
            let progress_weak = window.as_weak();
            let busy = Arc::clone(&download_busy);
            let cancelled = Arc::clone(&download_cancelled);
            let ready = Arc::clone(&download_ready);
            *download_worker.borrow_mut() = Some(std::thread::spawn(move || {
                let result = iriscope_updater::download(&release, &cancelled, &|percent| {
                    let _ = progress_weak.upgrade_in_event_loop(move |window| {
                        window
                            .global::<AppState>()
                            .set_update_progress(i32::from(percent));
                    });
                });
                let result = result.map(|update| {
                    *ready
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(update);
                });
                let _ = weak.upgrade_in_event_loop(move |window| {
                    busy.store(false, Ordering::Release);
                    let state = window.global::<AppState>();
                    state.set_update_busy(false);
                    match result {
                        Ok(()) => {
                            state.set_update_ready(true);
                            state.set_update_message(
                                "Mise à jour vérifiée. Prête à installer et redémarrer.".into(),
                            );
                        }
                        Err(error) => state.set_update_message(
                            format!("Téléchargement interrompu : {error}").into(),
                        ),
                    }
                });
            }));
        });
        let cancel = Arc::clone(&cancelled);
        state.on_cancel_update(move || cancel.store(true, Ordering::Release));
        let weak = window.as_weak();
        let install_requested = Rc::clone(&requested);
        let install_target = target.clone();
        state.on_install_update(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let state = window.global::<AppState>();
            if !state.get_update_ready()
                || state.get_update_busy()
                || !install_allowed(&window)
            {
                return;
            }
            let capture_directory = std::path::PathBuf::from(window.get_settings_capture_directory().as_str());
            if install_target.as_ref().is_some_and(|target| target.contains(&capture_directory)) {
                window.global::<AppState>().set_update_message("Déplacez d’abord le dossier des captures hors du dossier de l’application, puis réessayez.".into());
                return;
            }
            install_requested.set(true);
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested);
        });
        let timer = slint::Timer::default();
        if target.is_some() {
            let weak = window.as_weak();
            timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_secs(10),
                move || {
                    if let Some(window) = weak.upgrade() {
                        window.global::<AppState>().invoke_check_update();
                    }
                },
            );
        }
        Self {
            target,
            ready,
            requested,
            cancelled,
            worker,
            _startup_timer: timer,
        }
    }

    pub(super) fn finish(&self) -> iriscope_updater::Result<()> {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.borrow_mut().take() {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
        if self.requested.get() {
            let update = self
                .ready
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or("Mise à jour téléchargée introuvable")?;
            update.launch(
                self.target
                    .as_ref()
                    .ok_or("Installation non prise en charge")?,
            )?;
        } else {
            // Clean a downloaded package even if its UI notification arrived during shutdown.
            self.ready
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downloaded_update() -> (MainWindow, Mutex<Option<&'static str>>) {
        software_platform::init();
        let window = MainWindow::new().unwrap();
        let state = window.global::<AppState>();
        state.set_update_available(true);
        state.set_update_version("0.4.4".into());
        state.set_update_ready(true);
        state.set_update_progress(100);
        (window, Mutex::new(Some("0.4.4")))
    }

    #[test]
    fn newer_release_discards_the_previously_downloaded_version() {
        let (window, ready) = downloaded_update();
        let state = window.global::<AppState>();

        apply_check_state(&state, &ready, Ok(Some("0.4.5")));

        assert!(ready.lock().unwrap().is_none());
        assert!(!state.get_update_ready());
        assert_eq!(state.get_update_progress(), 0);
        assert!(state.get_update_available());
        assert_eq!(state.get_update_version(), "0.4.5");
        assert_eq!(state.get_update_message(), "Version 0.4.5 disponible.");
    }

    #[test]
    fn up_to_date_result_clears_the_download_and_available_version() {
        let (window, ready) = downloaded_update();
        let state = window.global::<AppState>();

        apply_check_state(&state, &ready, Ok(None));

        assert!(ready.lock().unwrap().is_none());
        assert!(!state.get_update_ready());
        assert_eq!(state.get_update_progress(), 0);
        assert!(!state.get_update_available());
        assert!(state.get_update_version().is_empty());
        assert_eq!(state.get_update_message(), "Iriscope est à jour.");
    }

    #[test]
    fn failed_check_keeps_the_known_version_and_its_download() {
        let (window, ready) = downloaded_update();
        let state = window.global::<AppState>();

        apply_check_state(&state, &ready, Err("hors ligne"));

        assert_eq!(*ready.lock().unwrap(), Some("0.4.4"));
        assert!(state.get_update_ready());
        assert!(state.get_update_available());
        assert_eq!(state.get_update_version(), "0.4.4");
        assert_eq!(
            state.get_update_message(),
            "Vérification impossible : hors ligne"
        );
    }

    #[test]
    fn restarting_download_prevents_installing_the_previous_package() {
        let (window, ready) = downloaded_update();
        let state = window.global::<AppState>();

        clear_download(&state, &ready);

        assert!(ready.lock().unwrap().is_none());
        assert!(!state.get_update_ready());
        assert_eq!(state.get_update_progress(), 0);
        assert!(state.get_update_available());
        assert_eq!(state.get_update_version(), "0.4.4");
    }

    #[test]
    fn install_request_cannot_close_an_active_recording_or_unsaved_notes() {
        software_platform::init();
        let window = MainWindow::new().unwrap();
        let _updates = UpdateController::install(&window);
        let count = Rc::new(Cell::new(0));
        let requests = Rc::clone(&count);
        window.window().on_close_requested(move || {
            requests.set(requests.get() + 1);
            slint::CloseRequestResponse::KeepWindowShown
        });
        let state = window.global::<AppState>();
        state.set_update_ready(true);
        state.set_is_recording(true);
        state.invoke_install_update();
        assert_eq!(count.get(), 0);
        state.set_is_recording(false);
        state.set_viewer_review_dirty(true);
        state.invoke_install_update();
        assert_eq!(count.get(), 0);
        state.set_viewer_review_dirty(false);
        state.set_storage_busy(true);
        state.invoke_install_update();
        assert_eq!(count.get(), 0);
        state.set_storage_busy(false);
        state.set_update_busy(true);
        state.invoke_install_update();
        assert_eq!(count.get(), 0);
        state.set_update_busy(false);
        state.invoke_install_update();
        assert_eq!(count.get(), 1);
    }
}
