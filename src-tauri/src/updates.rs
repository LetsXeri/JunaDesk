use crate::{model::Result, update_notifications};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{ipc::Channel, menu::MenuItem, AppHandle, Emitter, Manager, State, Wry};
use tauri_plugin_updater::{Error as UpdaterError, UpdaterExt};

const LATEST: &str = "https://api.github.com/repos/LetsXeri/JunaDesk/releases/latest";
const RELEASE_PAGE: &str = "https://github.com/LetsXeri/JunaDesk/releases/tag/v";
const DOWNLOADS: &str = "https://github.com/LetsXeri/JunaDesk/releases/download/v";
const MANIFEST: &str = "latest.json";
const MIN_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
struct Cache {
    automatic_checks: bool,
    system_notifications: bool,
    last_attempt: Option<u64>,
    available_version: Option<Version>,
    release_notes: Option<String>,
    installable: bool,
    last_notified: Option<Version>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            automatic_checks: true,
            system_notifications: false,
            last_attempt: None,
            available_version: None,
            release_notes: None,
            installable: false,
            last_notified: None,
        }
    }
}

impl Cache {
    fn load(path: &Path, installed: &Version) -> Self {
        let mut cache: Self = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        cache.available_version = cache
            .available_version
            .filter(|version| eligible(version, installed));
        cache.installable &= cache.available_version.is_some();
        if cache.available_version.is_none() {
            cache.release_notes = None;
        }
        cache
    }

    fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().ok_or("Update-Datenordner fehlt.")?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(storage_error)?;
        serde_json::to_writer(&mut file, self).map_err(storage_error)?;
        file.as_file().sync_all().map_err(storage_error)?;
        file.persist(path).map_err(storage_error)?;
        Ok(())
    }

    fn automatic_check_due(&self, now: u64) -> bool {
        self.automatic_checks
            && self
                .last_attempt
                .is_none_or(|last| now.saturating_sub(last) >= MIN_INTERVAL.as_secs())
    }

    fn pending_notification(&self) -> Option<&Version> {
        self.available_version.as_ref().filter(|version| {
            self.system_notifications
                && self
                    .last_notified
                    .as_ref()
                    .is_none_or(|last| version.cmp_precedence(last).is_gt())
        })
    }

    fn apply_release(&mut self, result: &Result<Option<Offer>>) {
        if let Ok(offer) = result {
            self.available_version = offer.as_ref().map(|offer| offer.version.clone());
            self.release_notes = offer.as_ref().and_then(|offer| offer.release_notes.clone());
            self.installable = offer.as_ref().is_some_and(|offer| offer.installable);
        }
    }
}

fn storage_error(error: impl std::fmt::Display) -> String {
    format!("Update-Einstellungen konnten nicht gespeichert werden. {error}")
}

struct Activity {
    cache: Cache,
    checking: bool,
    installing: bool,
}

pub struct Updates {
    path: PathBuf,
    installed: Version,
    activity: Mutex<Activity>,
    client: Result<reqwest::Client>,
    menu: MenuItem<Wry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    installed_version: String,
    available_version: Option<String>,
    release_notes: Option<String>,
    automatic_checks: bool,
    system_notifications: bool,
    checking: bool,
    installing: bool,
    installable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateOption {
    AutomaticChecks,
    SystemNotifications,
}

#[derive(Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum UpdateProgress {
    Preparing,
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying,
    Installing,
}

struct Installation<'a> {
    updates: &'a Updates,
    app: &'a AppHandle,
}

impl Drop for Installation<'_> {
    fn drop(&mut self) {
        self.updates.activity.lock().unwrap().installing = false;
        self.updates.publish(self.app);
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct Offer {
    version: Version,
    release_notes: Option<String>,
    installable: bool,
}

fn eligible(version: &Version, installed: &Version) -> bool {
    version.pre.is_empty() && version.cmp_precedence(installed).is_gt()
}

fn release_offer(
    release: Release,
    installed: &Version,
    platform: Option<(&str, &str)>,
) -> Option<Offer> {
    let (platform, extension) = platform?;
    if release.draft || release.prerelease || release.published_at.is_none() {
        return None;
    }
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    let uploaded = |name: &str| {
        release
            .assets
            .iter()
            .any(|asset| asset.name == name && asset.state == "uploaded" && asset.size > 0)
    };
    (eligible(&version, installed)
        && uploaded(&format!(
            "JunaDesk-{}-{platform}.{extension}",
            release.tag_name
        )))
    .then(|| {
        let package = update_package(&version, platform);
        Offer {
            version,
            installable: uploaded(MANIFEST) && uploaded(&package),
            release_notes: release
                .body
                .as_deref()
                .map(str::trim)
                .filter(|notes| !notes.is_empty())
                .map(str::to_owned),
        }
    })
}

fn update_package(version: &Version, platform: &str) -> String {
    let suffix = if platform == "macOS-arm64" {
        "app.tar.gz"
    } else {
        "exe"
    };
    format!("JunaDesk-v{version}-{platform}.{suffix}")
}

fn platform() -> Option<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(("macOS-arm64", "dmg")),
        ("windows", "x86_64") => Some(("Windows-x64", "exe")),
        _ => None,
    }
}

impl Updates {
    fn status(&self) -> UpdateStatus {
        let activity = self.activity.lock().unwrap();
        UpdateStatus {
            installed_version: self.installed.to_string(),
            available_version: activity
                .cache
                .available_version
                .as_ref()
                .map(ToString::to_string),
            automatic_checks: activity.cache.automatic_checks,
            release_notes: activity.cache.release_notes.clone(),
            system_notifications: activity.cache.system_notifications,
            checking: activity.checking,
            installing: activity.installing,
            installable: activity.cache.installable && !tauri::is_dev(),
        }
    }

    fn publish(&self, app: &AppHandle) {
        let status = self.status();
        let caption = status.available_version.as_ref().map_or_else(
            || "Updates …".to_owned(),
            |version| format!("JunaDesk {version} ist verfügbar"),
        );
        let _ = self.menu.set_text(caption);
        let _ = app.emit("updates-changed", ());
    }

    async fn latest(&self) -> Result<Option<Offer>> {
        let client = self.client.as_ref().map_err(Clone::clone)?;
        let response = client
            .get(LATEST)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|_| "GitHub ist zurzeit nicht erreichbar.".to_owned())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let release = response
            .error_for_status()
            .map_err(|error| {
                format!(
                    "GitHub konnte die Update-Anfrage nicht beantworten (HTTP {}).",
                    error.status().map_or(0, |status| status.as_u16())
                )
            })?
            .json()
            .await
            .map_err(|_| "GitHub hat ungültige Update-Daten geliefert.".to_owned())?;
        Ok(release_offer(release, &self.installed, platform()))
    }

    async fn check(&self, app: &AppHandle, manual: bool) -> Result<UpdateStatus> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        {
            let mut activity = self.activity.lock().unwrap();
            if activity.checking
                || activity.installing
                || (!manual && !activity.cache.automatic_check_due(now))
            {
                drop(activity);
                return Ok(self.status());
            }
            activity.checking = true;
            activity.cache.last_attempt = Some(now);
            let _ = activity.cache.save(&self.path);
        }
        self.publish(app);
        let result = self.latest().await;
        {
            let mut activity = self.activity.lock().unwrap();
            activity.cache.apply_release(&result);
            let _ = activity.cache.save(&self.path);
        }
        if result.is_ok() {
            self.notify(app).await;
        }
        self.activity.lock().unwrap().checking = false;
        self.publish(app);
        result.map(|_| self.status())
    }

    async fn notify(&self, app: &AppHandle) {
        let pending = self
            .activity
            .lock()
            .unwrap()
            .cache
            .pending_notification()
            .is_some();
        if !pending || !update_notifications::allowed(app).await {
            return;
        }
        let version = {
            let mut activity = self.activity.lock().unwrap();
            let Some(version) = activity.cache.pending_notification().cloned() else {
                return;
            };
            let mut next = activity.cache.clone();
            next.last_notified = Some(version.clone());
            if next.save(&self.path).is_err() {
                return;
            }
            activity.cache = next;
            version
        };
        update_notifications::show(app, &version);
    }
}

pub fn init(app: &AppHandle, root: &Path, menu: MenuItem<Wry>) {
    let path = root.join("updates.json");
    let installed = app.package_info().version.clone();
    let cache = Cache::load(&path, &installed);
    let client = reqwest::Client::builder()
        .user_agent("JunaDesk")
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Der Update-Netzwerkdienst konnte nicht gestartet werden.".into());
    app.manage(Updates {
        path,
        installed,
        activity: Mutex::new(Activity {
            cache,
            checking: false,
            installing: false,
        }),
        client,
        menu,
    });
    app.state::<Updates>().publish(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let _ = app.state::<Updates>().check(&app, false).await;
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        }
    });
}

#[tauri::command]
pub fn get_update_status(updates: State<'_, Updates>) -> UpdateStatus {
    updates.status()
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) -> Result<UpdateStatus> {
    app.state::<Updates>().check(&app, true).await
}

#[tauri::command]
pub async fn set_update_option(
    app: AppHandle,
    option: UpdateOption,
    enabled: bool,
) -> Result<UpdateStatus> {
    if matches!(option, UpdateOption::SystemNotifications) && enabled {
        update_notifications::request(&app).await?;
    }
    let updates = app.state::<Updates>();
    {
        let mut activity = updates.activity.lock().unwrap();
        let mut next = activity.cache.clone();
        match option {
            UpdateOption::AutomaticChecks => next.automatic_checks = enabled,
            UpdateOption::SystemNotifications => next.system_notifications = enabled,
        }
        next.save(&updates.path)?;
        activity.cache = next;
    }
    updates.publish(&app);
    if matches!(option, UpdateOption::AutomaticChecks) && enabled {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = app.state::<Updates>().check(&app, false).await;
        });
    }
    Ok(updates.status())
}

#[tauri::command]
pub fn open_update_release(app: AppHandle) -> Result<()> {
    if let Some(version) = app.state::<Updates>().status().available_version {
        tauri_plugin_opener::open_url(format!("{RELEASE_PAGE}{version}"), None::<&str>)
            .map_err(|_| "Die GitHub-Seite konnte nicht geöffnet werden.".into())
    } else {
        Err("Derzeit ist kein Update verfügbar.".into())
    }
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    version: String,
    progress: Channel<UpdateProgress>,
) -> Result<()> {
    let version = Version::parse(&version).map_err(|_| "Ungültige Update-Version.")?;
    let (platform, _) = platform().ok_or("Updates in der App werden hier nicht unterstützt.")?;
    let state = app.state::<Updates>();
    {
        let mut activity = state.activity.lock().unwrap();
        if activity.installing || activity.checking {
            return Err("Eine Update-Aktion läuft bereits.".into());
        }
        if tauri::is_dev()
            || !activity.cache.installable
            || activity.cache.available_version.as_ref() != Some(&version)
        {
            return Err("Für diese Version steht derzeit kein Update in der App bereit.".into());
        }
        activity.installing = true;
    }
    let _installation = Installation {
        updates: &state,
        app: &app,
    };
    state.publish(&app);
    let _ = progress.send(UpdateProgress::Preparing);

    // Resolve the stable release again before downloading the package shown to the user.
    let latest = state.latest().await;
    {
        let mut activity = state.activity.lock().unwrap();
        activity.cache.apply_release(&latest);
        let _ = activity.cache.save(&state.path);
    }
    if !latest?.is_some_and(|offer| offer.installable && offer.version == version) {
        return Err("Das verfügbare Update hat sich geändert. Prüfe erneut auf Updates.".into());
    }
    let base = format!("{DOWNLOADS}{version}/");
    let endpoint = format!("{base}{MANIFEST}")
        .parse()
        .map_err(|_| "Ungültige Update-Adresse.")?;
    let builder = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|_| "Der Updater konnte nicht gestartet werden.")?;
    // ShellExecute can reject the installer. Keep the window and tray alive
    // if that happens so the user can see the error and retry.
    #[cfg(windows)]
    let builder = builder.on_before_exit(|| {});
    let updater = builder
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "Der Updater konnte nicht gestartet werden.")?;
    let mut update = updater
        .check()
        .await
        .map_err(|_| "Die signierten Update-Daten konnten nicht geladen werden.")?
        .ok_or("Das Update-Paket ist nicht verfügbar.")?;
    if update.version != version.to_string()
        || update.download_url.as_str() != format!("{base}{}", update_package(&version, platform))
    {
        return Err("Das Update-Paket passt nicht zur angekündigten Version.".into());
    }
    update.timeout = Some(Duration::from_secs(15 * 60));
    let mut downloaded = 0u64;
    let mut last_report = None::<Instant>;
    let _ = progress.send(UpdateProgress::Downloading {
        downloaded,
        total: None,
    });
    let bytes = update
        .download(
            |size, total| {
                downloaded += size as u64;
                if last_report.is_none_or(|last| last.elapsed() >= Duration::from_millis(150)) {
                    last_report = Some(Instant::now());
                    let _ = progress.send(UpdateProgress::Downloading { downloaded, total });
                }
            },
            || { let _ = progress.send(UpdateProgress::Verifying); },
        )
        .await
        .map_err(|error| match error {
            UpdaterError::Minisign(_)
            | UpdaterError::Base64(_)
            | UpdaterError::SignatureUtf8(_)
            | UpdaterError::MissingSignedVersion
            | UpdaterError::SignedVersionMismatch { .. } =>
                "Die Update-Signatur konnte nicht bestätigt werden.".to_owned(),
            _ => "Das Update konnte nicht heruntergeladen werden. Prüfe deine Verbindung und versuche es erneut.".to_owned(),
        })?;

    let _ = progress.send(UpdateProgress::Installing);
    let store = app.state::<Arc<crate::Store>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = store.media_operation.lock().unwrap();
        update.install(&bytes)
    })
    .await
    .map_err(|_| "Die Update-Installation konnte nicht gestartet werden.")?
    .map_err(|error| format!("Das Update konnte nicht installiert werden. {error}"))?;
    #[cfg(not(windows))]
    app.request_restart();
    Ok(())
}

pub fn open_from_tray(app: &AppHandle) {
    crate::show_window(app);
    let _ = app.emit("show-update-settings", ());
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: Option<(&str, &str)> = Some(("macOS-arm64", "dmg"));
    const WINDOWS: Option<(&str, &str)> = Some(("Windows-x64", "exe"));

    fn published(tag: &str, platform: (&str, &str)) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease: false,
            published_at: Some("2026-11-20T12:00:00Z".to_owned()),
            body: None,
            assets: vec![Asset {
                name: format!("JunaDesk-{tag}-{}.{}", platform.0, platform.1),
                state: "uploaded".into(),
                size: 1024,
            }],
        }
    }

    #[test]
    fn accepts_only_newer_stable_releases_for_the_installed_platform() {
        let installed = Version::parse("0.9.0").unwrap();
        let version = Version::parse("0.10.0").unwrap();
        assert_eq!(
            release_offer(published("v0.10.0", MAC.unwrap()), &installed, MAC),
            Some(Offer {
                version: version.clone(),
                installable: false,
                release_notes: None,
            })
        );
        assert_eq!(
            release_offer(published("v0.10.0", WINDOWS.unwrap()), &installed, WINDOWS),
            Some(Offer {
                version,
                installable: false,
                release_notes: None,
            })
        );
        for tag in ["v0.9.0", "v0.8.0", "v0.10.0-rc.1", "not-a-version"] {
            assert_eq!(
                release_offer(published(tag, MAC.unwrap()), &installed, MAC),
                None
            );
        }
        assert_eq!(
            release_offer(published("v0.10.0", MAC.unwrap()), &installed, WINDOWS),
            None
        );
        assert_eq!(
            release_offer(published("v0.10.0", MAC.unwrap()), &installed, None),
            None
        );
    }

    #[test]
    fn ignores_github_previews_drafts_and_incomplete_uploads() {
        let installed = Version::parse("0.9.0").unwrap();
        let candidate = || published("v0.10.0", MAC.unwrap());
        let missing_assets = Release {
            assets: vec![],
            ..candidate()
        };
        let mut upload = candidate();
        upload.assets[0].size = 0;
        for release in [
            Release {
                prerelease: true,
                ..candidate()
            },
            Release {
                draft: true,
                ..candidate()
            },
            Release {
                published_at: None,
                ..candidate()
            },
            missing_assets,
            upload,
        ] {
            assert_eq!(release_offer(release, &installed, MAC), None);
        }
    }

    #[test]
    fn only_complete_updater_assets_enable_in_app_installation() {
        let installed = Version::parse("0.9.0").unwrap();
        let version = Version::parse("0.10.0").unwrap();
        for platform in [MAC.unwrap(), WINDOWS.unwrap()] {
            let candidate = |manifest_size, package_size| {
                let mut release = published("v0.10.0", platform);
                release.assets.extend([
                    Asset {
                        name: MANIFEST.into(),
                        state: "uploaded".into(),
                        size: manifest_size,
                    },
                    Asset {
                        name: update_package(&version, platform.0),
                        state: "uploaded".into(),
                        size: package_size,
                    },
                ]);
                release
            };
            let with_manifest = release_offer(candidate(42, 1024), &installed, Some(platform));
            assert!(with_manifest.unwrap().installable);
            let without_manifest = release_offer(candidate(0, 1024), &installed, Some(platform));
            assert!(!without_manifest.unwrap().installable);
        }
        let mut incomplete_mac = published("v0.10.0", MAC.unwrap());
        incomplete_mac.assets.push(Asset {
            name: MANIFEST.into(),
            state: "uploaded".into(),
            size: 42,
        });
        assert!(
            !release_offer(incomplete_mac, &installed, MAC)
                .unwrap()
                .installable
        );
    }

    #[test]
    fn keeps_cached_update_on_network_failure_and_clears_with_successful_check() {
        let offer = Version::parse("0.10.0").unwrap();
        let mut cache = Cache {
            available_version: Some(offer.clone()),
            release_notes: Some("**Neues Update**".into()),
            installable: true,
            ..Cache::default()
        };
        assert!(cache.pending_notification().is_none());
        cache.apply_release(&Err("offline".into()));
        assert_eq!(cache.available_version, Some(offer));
        assert_eq!(cache.release_notes.as_deref(), Some("**Neues Update**"));
        assert!(cache.installable);
        cache.apply_release(&Ok(None));
        assert!(cache.available_version.is_none());
        assert!(cache.release_notes.is_none());
        assert!(!cache.installable);
    }

    #[test]
    fn persists_the_check_interval_and_notification_deduplication_across_restarts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("updates.json");
        let installed = Version::parse("0.9.0").unwrap();
        let available = Version::parse("0.10.0").unwrap();
        let cache = Cache {
            system_notifications: true,
            last_attempt: Some(1000),
            available_version: Some(available.clone()),
            release_notes: Some("Alte Versionshinweise".into()),
            installable: true,
            last_notified: Some(available.clone()),
            ..Cache::default()
        };
        cache.save(&path).unwrap();
        let mut reopened = Cache::load(&path, &installed);
        assert!(reopened.pending_notification().is_none());
        assert!(!reopened.automatic_check_due(1000 + MIN_INTERVAL.as_secs() - 1));
        assert!(reopened.automatic_check_due(1000 + MIN_INTERVAL.as_secs()));
        reopened.automatic_checks = false;
        assert!(!reopened.automatic_check_due(1000 + MIN_INTERVAL.as_secs()));
        let mut release = published("v0.11.0", MAC.unwrap());
        release.body = Some("  ## Neu\n\n- Flüssigere Wiedergabe\n".into());
        reopened.apply_release(&Ok(release_offer(release, &installed, MAC)));
        assert_eq!(
            reopened.release_notes.as_deref(),
            Some("## Neu\n\n- Flüssigere Wiedergabe")
        );
        assert!(reopened.pending_notification().is_some());
        let upgraded = Cache::load(&path, &available);
        assert!(upgraded.available_version.is_none());
        assert!(upgraded.release_notes.is_none());
        assert!(!upgraded.installable);
    }
}
