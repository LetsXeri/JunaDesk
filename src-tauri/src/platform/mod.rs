#[cfg(not(any(target_os = "macos", windows)))]
use crate::model::{Observation, Playback, Result};
#[cfg(not(any(target_os = "macos", windows)))]
use std::{collections::BTreeMap, path::Path};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Desktop;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Desktop;

#[cfg(not(any(target_os = "macos", windows)))]
pub struct Desktop;

#[cfg(not(any(target_os = "macos", windows)))]
impl Desktop {
    pub fn new(_: &Path) -> Self {
        Self
    }
    pub fn observe(&mut self) -> Result<Observation> {
        Err("JunaDesk unterstützt macOS und Windows.".into())
    }
    pub fn apply(&mut self, _: &BTreeMap<String, Playback>) -> Result<()> {
        Ok(())
    }
}
