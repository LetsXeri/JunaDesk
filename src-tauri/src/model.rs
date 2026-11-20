use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Write, path::Path};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wallpaper {
    pub id: String,
    pub name: String,
    pub extension: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub fullscreen_pause: bool,
    pub battery_pause: bool,
    pub autostart: bool,
    pub fps: Option<u8>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fullscreen_pause: true,
            battery_pause: false,
            autostart: false,
            fps: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub version: u32,
    pub wallpapers: Vec<Wallpaper>,
    pub assignments: BTreeMap<String, String>,
    #[serde(default)]
    pub display_names: BTreeMap<String, String>,
    pub settings: Settings,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            wallpapers: Vec::new(),
            assignments: BTreeMap::new(),
            display_names: BTreeMap::new(),
            settings: Settings::default(),
        }
    }
}

impl Library {
    pub fn load(root: &Path) -> Result<Self> {
        let data = match fs::read(root.join("library.json")) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error.to_string()),
        };
        let library: Self = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        if library.version != 1
            || library.wallpapers.iter().any(|item| {
                Uuid::parse_str(&item.id).is_err()
                    || !matches!(item.extension.as_str(), "mp4" | "webm" | "gif")
            })
            || !matches!(library.settings.fps, None | Some(15 | 30))
        {
            return Err("Die Bibliothek enthält nicht unterstützte Daten.".into());
        }
        Ok(library)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let mut file = NamedTempFile::new_in(root).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut file, self).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(root.join("library.json"))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub monitors: Vec<Monitor>,
    pub on_battery: bool,
    #[serde(default)]
    pub has_battery: Option<bool>,
    #[serde(default)]
    pub rendered: BTreeMap<String, Playback>,
    pub errors: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Playback {
    pub path: std::path::PathBuf,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub monitors: Vec<Monitor>,
    pub on_battery: bool,
    pub has_battery: Option<bool>,
    pub ready_wallpapers: BTreeMap<String, String>,
    pub manual_pause: bool,
    pub errors: BTreeMap<String, String>,
    pub pause_reasons: BTreeMap<String, &'static str>,
    pub desktop_error: Option<String>,
}
