#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod media;
mod model;
mod platform;
mod update_notifications;
mod updates;

use model::{Library, Observation, Playback, Result, Settings, Status};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

struct Store {
    root: PathBuf,
    media_root: PathBuf,
    tools: media::Tools,
    library: Mutex<Library>,
    status: Mutex<Status>,
    media_operation: Mutex<()>,
    manual_pause: AtomicBool,
    stopping: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Serialize)]
struct Snapshot {
    library: Library,
    status: Status,
    previews: BTreeMap<String, PathBuf>,
}

#[derive(Serialize)]
struct ImportResult {
    file: String,
    error: Option<String>,
}

fn snapshot(store: &Store) -> Snapshot {
    let library = store.library.lock().unwrap().clone();
    let previews = library
        .wallpapers
        .iter()
        .map(|wallpaper| {
            (
                wallpaper.id.clone(),
                store.media_root.join(&wallpaper.id).join("preview.jpg"),
            )
        })
        .collect();
    Snapshot {
        library,
        status: store.status.lock().unwrap().clone(),
        previews,
    }
}

fn edit(store: &Store, change: impl FnOnce(&mut Library) -> Result<()>) -> Result<()> {
    let mut current = store.library.lock().unwrap();
    let mut next = current.clone();
    change(&mut next)?;
    next.save(&store.root)?;
    *current = next;
    Ok(())
}

fn changed(app: &AppHandle) {
    let _ = app.emit("library-changed", ());
}

#[tauri::command]
fn wallpaper_preview(store: State<'_, Arc<Store>>, id: String) -> Result<PathBuf> {
    let library = store.library.lock().unwrap();
    if !library.wallpapers.iter().any(|item| item.id == id) {
        return Err("Das Wallpaper wurde nicht gefunden.".into());
    }
    let path = media::rendition(&store.media_root.join(id), None);
    path.is_file().then_some(path).ok_or_else(|| {
        "Die Vorschau-Datei fehlt. Importiere das Wallpaper bei Bedarf erneut.".into()
    })
}

#[tauri::command]
fn reveal_wallpaper(store: State<'_, Arc<Store>>, id: String) -> Result<()> {
    let path = {
        let library = store.library.lock().unwrap();
        let item = library
            .wallpapers
            .iter()
            .find(|item| item.id == id)
            .ok_or("Das Wallpaper wurde nicht gefunden.")?;
        store
            .media_root
            .join(id)
            .join(format!("original.{}", item.extension))
    };
    if !path.is_file() {
        return Err("Die verwaltete Datei wurde nicht gefunden.".into());
    }
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|_| "Die Datei konnte im Dateimanager nicht angezeigt werden.".into())
}

#[tauri::command]
fn get_snapshot(store: State<'_, Arc<Store>>) -> Snapshot {
    snapshot(&store)
}

#[tauri::command]
async fn import_wallpapers(app: AppHandle, paths: Vec<PathBuf>) -> Result<Vec<ImportResult>> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<Arc<Store>>();
        let _operation = store.media_operation.lock().unwrap();
        let fps = store.library.lock().unwrap().settings.fps;
        paths
            .into_iter()
            .map(|source| {
                let file = source
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let result = (|| -> Result<()> {
                    let (wallpaper, staging) =
                        media::import(&store.tools, &store.media_root, &source, fps)?;
                    let destination = store.media_root.join(&wallpaper.id);
                    fs::rename(staging.path(), &destination).map_err(|e| e.to_string())?;
                    if let Err(error) = edit(&store, |library| {
                        library.wallpapers.push(wallpaper);
                        Ok(())
                    }) {
                        let _ = fs::remove_dir_all(destination);
                        return Err(error);
                    }
                    changed(&app);
                    Ok(())
                })();
                ImportResult {
                    file,
                    error: result.err(),
                }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_wallpaper(app: AppHandle, id: String, name: String) -> Result<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("Bitte einen Namen mit 1 bis 100 Zeichen eingeben.".into());
    }
    edit(&app.state::<Arc<Store>>(), |library| {
        let wallpaper = library
            .wallpapers
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or("Das Wallpaper wurde nicht gefunden.")?;
        wallpaper.name = name.to_owned();
        Ok(())
    })?;
    changed(&app);
    Ok(())
}

#[tauri::command]
fn assign_wallpaper(
    app: AppHandle,
    monitor_id: String,
    wallpaper_id: Option<String>,
) -> Result<()> {
    let store = app.state::<Arc<Store>>();
    let monitor = store
        .status
        .lock()
        .unwrap()
        .monitors
        .iter()
        .find(|item| item.id == monitor_id)
        .cloned();
    edit(&store, |library| {
        if let Some(id) = wallpaper_id {
            let monitor = monitor.ok_or("Der Bildschirm ist nicht mehr verbunden.")?;
            if !library.wallpapers.iter().any(|item| item.id == id) {
                return Err("Das Wallpaper wurde nicht gefunden.".into());
            }
            library.assignments.insert(monitor_id.clone(), id);
            library.display_names.insert(monitor_id, monitor.name);
        } else {
            library.assignments.remove(&monitor_id);
            library.display_names.remove(&monitor_id);
        }
        Ok(())
    })?;
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn delete_wallpaper(app: AppHandle, id: String) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<Arc<Store>>();
        let _operation = store.media_operation.lock().unwrap();
        edit(&store, |library| {
            if !library.wallpapers.iter().any(|item| item.id == id) {
                return Err("Das Wallpaper wurde nicht gefunden.".into());
            }
            library.wallpapers.retain(|item| item.id != id);
            library.assignments.retain(|_, item| *item != id);
            library.display_names.retain(|key, _| library.assignments.contains_key(key));
            Ok(())
        })?;
        changed(&app);
        let directory = store.media_root.join(id);
        // Windows may keep the current video open until the desktop thread releases it.
        for _ in 0..20 {
            match fs::remove_dir_all(&directory) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => thread::sleep(Duration::from_millis(250)),
            }
        }
        Err("Wallpaper und Zuweisungen wurden entfernt. Die übrigen Dateien werden beim nächsten Start erneut zur Löschung vorgemerkt.".into())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn set_fps(app: AppHandle, fps: Option<u8>) -> Result<()> {
    if !matches!(fps, None | Some(15 | 30)) {
        return Err("Unterstützt werden 15 FPS, 30 FPS oder kein Limit.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<Arc<Store>>();
        let _operation = store.media_operation.lock().unwrap();
        let library = store.library.lock().unwrap().clone();
        if library.settings.fps == fps {
            return Ok(());
        }
        for wallpaper in &library.wallpapers {
            media::prepare_limit(
                &store.tools,
                &store.media_root.join(&wallpaper.id),
                wallpaper,
                fps,
            )
            .map_err(|error| format!("„{}“: {error}", wallpaper.name))?;
        }
        edit(&store, |library| {
            library.settings.fps = fps;
            Ok(())
        })?;
        changed(&app);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn set_autostart(app: &AppHandle, enabled: bool) -> Result<()> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_option(app: AppHandle, option: String, enabled: bool) -> Result<()> {
    let store = app.state::<Arc<Store>>();
    let old_autostart = store.library.lock().unwrap().settings.autostart;
    if option == "autostart" && old_autostart != enabled {
        set_autostart(&app, enabled)?;
    }
    let result = edit(&store, |library| {
        match option.as_str() {
            "autostart" => library.settings.autostart = enabled,
            "batteryPause" => library.settings.battery_pause = enabled,
            "fullscreenPause" => library.settings.fullscreen_pause = enabled,
            _ => return Err("Unbekannte Einstellung.".into()),
        }
        Ok(())
    });
    if result.is_err() && option == "autostart" && old_autostart != enabled {
        let _ = set_autostart(&app, old_autostart);
    }
    result?;
    changed(&app);
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_pause(app: &AppHandle, pause_item: &MenuItem<tauri::Wry>) {
    let store = app.state::<Arc<Store>>();
    let paused = !store.manual_pause.fetch_xor(true, Ordering::AcqRel);
    let _ = pause_item.set_text(if paused { "Fortsetzen" } else { "Pause" });
}

fn desktop_loop(app: AppHandle, store: Arc<Store>, resources: PathBuf) {
    let mut desktop = platform::Desktop::new(&resources);
    let mut observation = Observation::default();
    while !store.stopping.load(Ordering::Acquire) {
        let observed_error = match desktop.observe() {
            Ok(value) => {
                observation = value;
                None
            }
            Err(error) => Some(error),
        };
        let manual_pause = store.manual_pause.load(Ordering::Acquire);
        let mut desired = BTreeMap::new();
        let mut pause_reasons = BTreeMap::new();
        let mut ready_wallpapers = BTreeMap::new();
        {
            let library = store.library.lock().unwrap();
            for monitor in &observation.monitors {
                let Some(id) = library.assignments.get(&monitor.id) else {
                    continue;
                };
                if !library
                    .wallpapers
                    .iter()
                    .any(|wallpaper| wallpaper.id == *id)
                {
                    continue;
                }
                let reason = pause_reason(
                    &library.settings,
                    manual_pause,
                    observation.on_battery,
                    monitor.fullscreen,
                );
                if let Some(reason) = reason {
                    pause_reasons.insert(monitor.id.clone(), reason);
                }
                let target = Playback {
                    path: media::rendition(&store.media_root.join(id), library.settings.fps),
                    paused: reason.is_some(),
                };
                if observation.rendered.get(&monitor.id) == Some(&target) {
                    ready_wallpapers.insert(monitor.id.clone(), id.clone());
                }
                desired.insert(monitor.id.clone(), target);
            }
        }
        let desktop_error = desktop.apply(&desired).err().or(observed_error);
        let status = Status {
            monitors: observation.monitors.clone(),
            on_battery: observation.on_battery,
            has_battery: observation.has_battery,
            ready_wallpapers,
            manual_pause,
            errors: observation.errors.clone(),
            pause_reasons,
            desktop_error,
        };
        {
            let mut latest = store.status.lock().unwrap();
            if status != *latest {
                *latest = status;
                let _ = app.emit("desktop-changed", &*latest);
            }
        }
        thread::park_timeout(Duration::from_millis(500));
    }
}

fn pause_reason(
    settings: &Settings,
    manual: bool,
    battery: bool,
    fullscreen: bool,
) -> Option<&'static str> {
    if manual {
        Some("Manuell pausiert")
    } else if settings.battery_pause && battery {
        Some("Akkubetrieb")
    } else if settings.fullscreen_pause && fullscreen {
        Some("Vollbild")
    } else {
        None
    }
}

fn cleanup_media(root: &Path, library: &Library) {
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (uuid::Uuid::parse_str(&name).is_ok()
            && !library.wallpapers.iter().any(|item| item.id == name))
            || name.starts_with(".junadesk-import-")
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn main() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            GetErrorMode, SetErrorMode, SEM_FAILCRITICALERRORS,
        };
        // Let the media processes report loader failures as exit codes.
        unsafe { SetErrorMode(GetErrorMode() | SEM_FAILCRITICALERRORS) };
    }

    let background = std::env::args().any(|argument| argument == "--background");
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(
        |app, arguments, _| {
            if !arguments.iter().any(|argument| argument == "--background") {
                show_window(app);
            }
        },
    ));
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_notification::init());
    let app = builder
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            wallpaper_preview,
            reveal_wallpaper,
            import_wallpapers,
            rename_wallpaper,
            assign_wallpaper,
            delete_wallpaper,
            set_fps,
            set_option,
            updates::get_update_status,
            updates::check_updates,
            updates::set_update_option,
            updates::open_update_release,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let root = app.path().app_local_data_dir()?;
            let media_root = root.join("wallpapers");
            fs::create_dir_all(&media_root)?;
            let resources = app.path().resource_dir()?.join("tools");
            let library = Library::load(&root)?;
            cleanup_media(&media_root, &library);
            let store = Arc::new(Store {
                root,
                media_root,
                library: Mutex::new(library),
                tools: media::Tools {
                    ffmpeg: media::binary(&resources, "ffmpeg"),
                    ffprobe: media::binary(&resources, "ffprobe"),
                },
                status: Mutex::new(Status {
                    monitors: Vec::new(),
                    on_battery: false,
                    has_battery: None,
                    ready_wallpapers: BTreeMap::new(),
                    manual_pause: false,
                    errors: BTreeMap::new(),
                    pause_reasons: BTreeMap::new(),
                    desktop_error: None,
                }),
                media_operation: Mutex::new(()),
                manual_pause: AtomicBool::new(false),
                stopping: AtomicBool::new(false),
                worker: Mutex::new(None),
            });
            app.manage(store.clone());
            let window_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .ok_or("Fensterkonfiguration für JunaDesk fehlt.")?;
            let mut window = tauri::WebviewWindowBuilder::from_config(app, window_config)?;
            if !background {
                // Give the first React render its startup state before any page script runs.
                let launch_id = uuid::Uuid::new_v4();
                window = window.initialization_script(format!(
                    "if (window === window.top) window.__JUNADESK_STARTUP__ = '{launch_id}';"
                ));
            }
            window.build()?;
            let open = MenuItem::with_id(app, "open", "Öffnen", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
            let update = MenuItem::with_id(app, "updates", "Updates …", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &pause, &update, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(cfg!(target_os = "macos"))
                .tooltip("JunaDesk")
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open" => show_window(app),
                    "pause" => toggle_pause(app, &pause),
                    "updates" => updates::open_from_tray(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if cfg!(windows)
                        && matches!(
                            event,
                            TrayIconEvent::Click {
                                button: MouseButton::Left,
                                button_state: MouseButtonState::Up,
                                ..
                            }
                        )
                    {
                        show_window(tray.app_handle());
                    }
                });
            #[cfg(target_os = "macos")]
            {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
                tray = tray.icon(icon).icon_as_template(true);
            }
            #[cfg(not(target_os = "macos"))]
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            updates::init(app.handle(), &store.root, update);
            let desktop_app = app.handle().clone();
            let desktop_store = store.clone();
            *store.worker.lock().unwrap() = Some(thread::spawn(move || {
                desktop_loop(desktop_app, desktop_store, resources)
            }));
            if !background {
                show_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("JunaDesk konnte nicht initialisiert werden");

    app.run(|app, event| {
        if matches!(event, RunEvent::Exit) {
            let store = app.state::<Arc<Store>>();
            store.stopping.store(true, Ordering::Release);
            let worker = store.worker.lock().unwrap().take();
            if let Some(worker) = worker {
                worker.thread().unpark();
                let _ = worker.join();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_priorities() {
        let mut settings = Settings::default();
        assert_eq!(pause_reason(&settings, false, true, false), None);
        settings.battery_pause = true;
        assert_eq!(
            pause_reason(&settings, true, true, true),
            Some("Manuell pausiert")
        );
        assert_eq!(
            pause_reason(&settings, false, true, true),
            Some("Akkubetrieb")
        );
        assert_eq!(
            pause_reason(&settings, false, false, true),
            Some("Vollbild")
        );
    }

    #[test]
    fn persistence_replaces_existing_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::default();
        library.save(directory.path()).unwrap();
        library
            .assignments
            .insert("display".into(), "wallpaper".into());
        library.settings.fps = Some(15);
        library.save(directory.path()).unwrap();
        let found = Library::load(directory.path()).unwrap();
        assert_eq!(found.assignments["display"], "wallpaper");
        assert_eq!(found.settings.fps, Some(15));
    }
}
