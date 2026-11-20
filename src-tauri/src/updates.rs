use crate::{model::Result, update_notifications};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{menu::MenuItem, AppHandle, Emitter, Manager, State, Wry};

const LATEST: &str = "https://api.github.com/repos/LetsXeri/JunaDesk/releases/latest";
const RELEASE_PAGE: &str = "https://github.com/LetsXeri/JunaDesk/releases/tag/v";
const MIN_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
struct Cache {
    automatic_checks: bool,
    system_notifications: bool,
    last_attempt: Option<u64>,
    available_version: Option<Version>,
    last_notified: Option<Version>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            automatic_checks: true,
            system_notifications: false,
            last_attempt: None,
            available_version: None,
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

    fn apply_release(&mut self, result: &Result<Option<Version>>) {
        if let Ok(version) = result {
            self.available_version.clone_from(version);
        }
    }
}

fn storage_error(error: impl std::fmt::Display) -> String {
    format!("Update-Einstellungen konnten nicht gespeichert werden. {error}")
}

struct Activity {
    cache: Cache,
    checking: bool,
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
    automatic_checks: bool,
    system_notifications: bool,
    checking: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateOption {
    AutomaticChecks,
    SystemNotifications,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
}

fn eligible(version: &Version, installed: &Version) -> bool {
    version.pre.is_empty() && version.cmp_precedence(installed).is_gt()
}

fn release_version(
    release: Release,
    installed: &Version,
    platform: Option<(&str, &str)>,
) -> Option<Version> {
    let (platform, extension) = platform?;
    if release.draft || release.prerelease || release.published_at.is_none() {
        return None;
    }
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    let expected = format!("JunaDesk-{}-{platform}.{extension}", release.tag_name);
    (eligible(&version, installed)
        && release
            .assets
            .iter()
            .any(|asset| asset.name == expected && asset.state == "uploaded" && asset.size > 0))
    .then_some(version)
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
            system_notifications: activity.cache.system_notifications,
            checking: activity.checking,
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

    async fn latest(&self) -> Result<Option<Version>> {
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
        Ok(release_version(release, &self.installed, platform()))
    }

    async fn check(&self, app: &AppHandle, manual: bool) -> Result<UpdateStatus> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        {
            let mut activity = self.activity.lock().unwrap();
            if activity.checking || (!manual && !activity.cache.automatic_check_due(now)) {
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

pub fn open_from_tray(app: &AppHandle) {
    if app.state::<Updates>().status().available_version.is_some()
        && open_update_release(app.clone()).is_ok()
    {
        return;
    }
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
            release_version(published("v0.10.0", MAC.unwrap()), &installed, MAC),
            Some(version.clone())
        );
        assert_eq!(
            release_version(published("v0.10.0", WINDOWS.unwrap()), &installed, WINDOWS),
            Some(version)
        );
        for tag in ["v0.9.0", "v0.8.0", "v0.10.0-rc.1", "not-a-version"] {
            assert_eq!(
                release_version(published(tag, MAC.unwrap()), &installed, MAC),
                None
            );
        }
        assert_eq!(
            release_version(published("v0.10.0", MAC.unwrap()), &installed, WINDOWS),
            None
        );
        assert_eq!(
            release_version(published("v0.10.0", MAC.unwrap()), &installed, None),
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
            assert_eq!(release_version(release, &installed, MAC), None);
        }
    }

    #[test]
    fn keeps_cached_update_on_network_failure_and_clears_with_successful_check() {
        let offer = Version::parse("0.10.0").unwrap();
        let mut cache = Cache {
            available_version: Some(offer.clone()),
            ..Cache::default()
        };
        assert!(cache.pending_notification().is_none());
        cache.apply_release(&Err("offline".into()));
        assert_eq!(cache.available_version, Some(offer));
        cache.apply_release(&Ok(None));
        assert!(cache.available_version.is_none());
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
        reopened.apply_release(&Ok(Some(Version::parse("0.11.0").unwrap())));
        assert!(reopened.pending_notification().is_some());
        let upgraded = Cache::load(&path, &available);
        assert!(upgraded.available_version.is_none());
    }
}
