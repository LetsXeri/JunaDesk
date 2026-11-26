use crate::model::Result;
use std::{
    os::windows::ffi::OsStrExt,
    path::Path,
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{implement, w, Interface, BSTR, PCWSTR},
    Win32::{
        Foundation::{HWND, RECT},
        Media::MediaFoundation::*,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
    },
};

pub(super) struct MediaRuntime;

impl MediaRuntime {
    pub(super) fn start() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(media_error)?;
            if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(media_error(error));
            }
        }
        Ok(Self)
    }
}

impl Drop for MediaRuntime {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

fn media_error(error: windows::core::Error) -> String {
    format!("Windows Media Foundation: {error}")
}

#[implement(IMFMediaEngineNotify)]
struct Notification(thread::Thread);

impl IMFMediaEngineNotify_Impl for Notification_Impl {
    fn EventNotify(&self, event: u32, _: usize, _: u32) -> windows::core::Result<()> {
        match MF_MEDIA_ENGINE_EVENT(event as i32) {
            MF_MEDIA_ENGINE_EVENT_ERROR
            | MF_MEDIA_ENGINE_EVENT_LOADEDDATA
            | MF_MEDIA_ENGINE_EVENT_CANPLAY
            | MF_MEDIA_ENGINE_EVENT_FORMATCHANGE => self.0.unpark(),
            _ => {}
        }
        Ok(())
    }
}

pub(super) struct Player {
    engine: IMFMediaEngine,
    video: IMFMediaEngineEx,
    started: Instant,
    paused: bool,
    geometry: Option<(i32, i32, u32, u32)>,
}

impl Player {
    pub(super) fn new(hwnd: HWND, path: &Path) -> Result<Self> {
        let filename: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        unsafe {
            // Open the native path directly. A URL would reinterpret '#' and
            // percent escapes before the local file resolver sees them.
            let stream = MFCreateFile(
                MF_ACCESSMODE_READ,
                MF_OPENMODE_FAIL_IF_NOT_EXIST,
                MF_FILEFLAGS_NONE,
                PCWSTR(filename.as_ptr()),
            )
            .map_err(media_error)?;
            if let Ok(metadata) = stream.cast::<IMFAttributes>() {
                metadata
                    .SetString(&MF_BYTESTREAM_CONTENT_TYPE, w!("video/mp4"))
                    .map_err(media_error)?;
            }
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 2).map_err(media_error)?;
            let attributes =
                attributes.ok_or("Media Foundation konnte nicht initialisiert werden.")?;
            let callback: IMFMediaEngineNotify = Notification(thread::current()).into();
            attributes
                .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &callback)
                .map_err(media_error)?;
            attributes
                .SetUINT64(&MF_MEDIA_ENGINE_PLAYBACK_HWND, hwnd.0 as usize as u64)
                .map_err(media_error)?;
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(media_error)?;
            let engine = factory
                .CreateInstance(0, &attributes)
                .map_err(media_error)?;
            let video = match engine.cast() {
                Ok(video) => video,
                Err(error) => {
                    let _ = engine.Shutdown();
                    return Err(media_error(error));
                }
            };
            let playback = Self {
                engine,
                video,
                started: Instant::now(),
                paused: true,
                geometry: None,
            };
            (|| {
                playback.engine.SetMuted(true)?;
                playback.engine.SetLoop(true)?;
                playback
                    .engine
                    .SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC)?;
                playback
                    .video
                    .SetSourceFromByteStream(&stream, &BSTR::from("file:///wallpaper.mp4"))
            })()
            .map_err(media_error)?;
            Ok(playback)
        }
    }

    pub(super) fn update(&mut self, paused: bool, width: i32, height: i32) -> Result<bool> {
        unsafe {
            if let Ok(error) = self.engine.GetError() {
                let code = error.GetErrorCode();
                if code != 0 {
                    let detail = error
                        .GetExtendedErrorCode()
                        .err()
                        .map(|error| error.to_string());
                    return Err(match detail {
                        Some(detail) => format!("Windows kann das Wallpaper nicht abspielen (Medienfehler {code}: {detail})."),
                        None => format!("Windows kann das Wallpaper nicht abspielen (Medienfehler {code})."),
                    });
                }
            }
            if self.engine.GetReadyState() < MF_MEDIA_ENGINE_READY_HAVE_CURRENT_DATA.0 as u16 {
                if self.geometry.is_none() && self.started.elapsed() > Duration::from_secs(15) {
                    return Err("Windows hat keine Videodaten bereitgestellt.".into());
                }
                return Ok(false);
            }
            if !self.engine.HasVideo().as_bool() {
                return Err("Die Datei enthält keine abspielbare Videospur.".into());
            }
            let (mut video_width, mut video_height) = (0, 0);
            self.engine
                .GetVideoAspectRatio(Some(&mut video_width), Some(&mut video_height))
                .map_err(media_error)?;
            if width <= 0 || height <= 0 || video_width == 0 || video_height == 0 {
                return Ok(false);
            }
            let geometry = (width, height, video_width, video_height);
            if self.geometry != Some(geometry) {
                let source = cover_rect(
                    video_width as f32 / video_height as f32,
                    width as f32 / height as f32,
                );
                let target = RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
                self.video
                    .UpdateVideoStream(Some(&source), Some(&target), None)
                    .map_err(media_error)?;
                self.geometry = Some(geometry);
            }
            if self.paused != paused {
                if paused {
                    self.engine.Pause()
                } else {
                    self.engine.Play()
                }
                .map_err(media_error)?;
                self.paused = paused;
            }
            Ok(true)
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}

fn cover_rect(video: f32, target: f32) -> MFVideoNormalizedRect {
    let x = (1.0 - (target / video).min(1.0)) / 2.0;
    let y = (1.0 - (video / target).min(1.0)) / 2.0;
    MFVideoNormalizedRect {
        left: x,
        top: y,
        right: 1.0 - x,
        bottom: 1.0 - y,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Host, Monitor, Surface, SurfaceWindow};
    use super::*;
    use crate::model::Playback;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*};

    #[test]
    fn native_player_loads_loops_and_pauses_an_h264_wallpaper() {
        let _runtime = MediaRuntime::start().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Test & Übung #1.mp4");
        std::fs::write(&path, include_bytes!("../../testdata/test-h264.mp4")).unwrap();
        // Use a visible stand-in for Explorer on the Windows CI desktop.
        let window = SurfaceWindow::new(unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                null(),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                320,
                240,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        });
        assert!(!window.hwnd.is_null());
        let host = Host {
            parent: window.hwnd,
            icons: null_mut(),
            background: null_mut(),
        };
        let monitor = Monitor {
            id: String::new(),
            name: String::new(),
            width: 320,
            height: 240,
            x: 0,
            y: 0,
            primary: true,
            fullscreen: false,
        };
        let mut video = Playback {
            path,
            paused: false,
        };
        let mut surface = Surface::new(host, &monitor, &video).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut previous = 0.0;
        loop {
            surface.update(host, &monitor, &video).unwrap();
            let current = unsafe { surface.player.engine.GetCurrentTime() };
            if surface.ready && previous > 0.65 && current + 0.4 < previous {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Media Foundation failed to loop the test video"
            );
            previous = current;
            thread::sleep(Duration::from_millis(20));
        }
        video.paused = true;
        surface.update(host, &monitor, &video).unwrap();
        assert!(unsafe { surface.player.engine.IsPaused().as_bool() });
        assert!(unsafe { surface.player.engine.GetMuted().as_bool() });
    }
}
