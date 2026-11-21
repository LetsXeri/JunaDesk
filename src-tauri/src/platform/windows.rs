use crate::{
    media,
    model::{Monitor, Observation, Playback, Result},
};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    mem::{size_of, zeroed},
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    ptr::{null, null_mut},
    sync::OnceLock,
    time::{Duration, Instant},
};
use uuid::Uuid;
use windows_sys::Win32::{
    Devices::Display::*,
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{JobObjects::*, LibraryLoader::GetModuleHandleW, Power::*},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

pub struct Desktop {
    mpv: PathBuf,
    host: Option<Host>,
    surfaces: BTreeMap<String, Surface>,
    monitors: Vec<Monitor>,
    errors: BTreeMap<String, String>,
    retries: BTreeMap<String, (PathBuf, Instant)>,
    next_attach: Instant,
}

#[derive(Clone, Copy, PartialEq)]
struct Host {
    parent: HWND,
    icons: HWND,
}

struct Surface {
    hwnd: HWND,
    job: HANDLE,
    child: Child,
    pipe: String,
    path: PathBuf,
    paused: bool,
    started: Instant,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn string(wide: &[u16]) -> String {
    String::from_utf16_lossy(&wide[..wide.iter().position(|c| *c == 0).unwrap_or(wide.len())])
}

impl Desktop {
    pub fn new(resources: &Path) -> Self {
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Self {
            mpv: media::binary(resources, "mpv"),
            host: None,
            surfaces: BTreeMap::new(),
            monitors: Vec::new(),
            errors: BTreeMap::new(),
            retries: BTreeMap::new(),
            next_attach: Instant::now(),
        }
    }

    pub fn observe(&mut self) -> Result<Observation> {
        unsafe {
            let mut message: MSG = zeroed();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        self.monitors = monitors();
        if self.monitors.is_empty() {
            return Err("Windows meldet keine verfügbaren Bildschirme.".into());
        }
        let mut power: SYSTEM_POWER_STATUS = unsafe { zeroed() };
        let power_available = unsafe { GetSystemPowerStatus(&mut power) != 0 };
        let on_battery = power_available && power.ACLineStatus == 0;
        Ok(Observation {
            monitors: self.monitors.clone(),
            on_battery,
            has_battery: match (power_available, power.BatteryFlag) {
                (false, _) | (_, 255) => None,
                (_, flags) => Some(flags & 128 == 0),
            },
            errors: self.errors.clone(),
            ..Observation::default()
        })
    }

    pub fn apply(&mut self, desired: &BTreeMap<String, Playback>) -> Result<()> {
        self.errors.retain(|id, _| desired.contains_key(id));
        self.retries.retain(|id, _| desired.contains_key(id));
        self.surfaces.retain(|id, surface| {
            desired
                .get(id)
                .is_some_and(|video| video.path == surface.path)
        });
        if desired.is_empty() {
            return Ok(());
        }
        if self.host.is_some_and(|host| unsafe {
            IsWindow(host.parent) == 0 || (!host.icons.is_null() && IsWindow(host.icons) == 0)
        }) {
            self.surfaces.clear();
            self.host = None;
            self.next_attach = Instant::now();
        }
        if self.host.is_none() && Instant::now() >= self.next_attach {
            self.next_attach = Instant::now() + Duration::from_secs(3);
            self.host = unsafe { desktop_host() };
        }
        let host = self
            .host
            .ok_or("Die Desktop-Ebene von Explorer wurde nicht gefunden.")?;
        for monitor in &self.monitors {
            let Some(video) = desired.get(&monitor.id) else {
                continue;
            };
            if self
                .retries
                .get(&monitor.id)
                .is_some_and(|(path, until)| *path == video.path && Instant::now() < *until)
            {
                continue;
            }
            let result = if let Some(surface) = self.surfaces.get_mut(&monitor.id) {
                surface.update(host, monitor, video)
            } else {
                Surface::new(host, monitor, video, &self.mpv).map(|surface| {
                    self.surfaces.insert(monitor.id.clone(), surface);
                })
            };
            match result {
                Ok(()) => {
                    self.errors.remove(&monitor.id);
                    self.retries.remove(&monitor.id);
                }
                Err(error) => {
                    self.surfaces.remove(&monitor.id);
                    self.retries.insert(
                        monitor.id.clone(),
                        (video.path.clone(), Instant::now() + Duration::from_secs(5)),
                    );
                    self.errors.insert(monitor.id.clone(), error);
                }
            }
        }
        Ok(())
    }
}

unsafe fn desktop_host() -> Option<Host> {
    let progman = FindWindowW(wide("Progman").as_ptr(), null());
    if progman.is_null() {
        return None;
    }
    // Explorer creates the wallpaper layer in response to this shell message.
    SendMessageTimeoutW(progman, 0x052c, 0x0d, 1, SMTO_ABORTIFHUNG, 1000, null_mut());
    let defview = wide("SHELLDLL_DefView");
    let worker = wide("WorkerW");
    let icons = FindWindowExW(progman, null_mut(), defview.as_ptr(), null());
    let background = FindWindowExW(progman, icons, worker.as_ptr(), null());
    if !icons.is_null() && !background.is_null() {
        return Some(Host {
            parent: progman,
            icons,
        });
    }

    let mut host = None;
    unsafe extern "system" fn find(window: HWND, data: LPARAM) -> BOOL {
        if !FindWindowExW(
            window,
            null_mut(),
            wide("SHELLDLL_DefView").as_ptr(),
            null(),
        )
        .is_null()
        {
            let background = FindWindowExW(null_mut(), window, wide("WorkerW").as_ptr(), null());
            if !background.is_null()
                && FindWindowExW(
                    background,
                    null_mut(),
                    wide("SHELLDLL_DefView").as_ptr(),
                    null(),
                )
                .is_null()
            {
                *(data as *mut Option<Host>) = Some(Host {
                    parent: background,
                    icons: null_mut(),
                });
                return 0;
            }
        }
        1
    }
    EnumWindows(Some(find), &mut host as *mut Option<Host> as LPARAM);
    host
}

const SURFACE_CLASS: *const u16 = windows_sys::w!("JunaDeskWallpaperSurface");
static SURFACE_CLASS_REGISTRATION: OnceLock<std::result::Result<(), u32>> = OnceLock::new();

fn window_error(message: &str, code: u32) -> String {
    format!("{message} (Win32-Fehler {code})")
}

fn create_surface_window(parent: HWND, layered: bool, width: i32, height: i32) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(null());
        SURFACE_CLASS_REGISTRATION
            .get_or_init(|| {
                // Layered windows are incompatible with CS_PARENTDC, CS_CLASSDC and CS_OWNDC.
                let class = WNDCLASSW {
                    lpfnWndProc: Some(DefWindowProcW),
                    hInstance: instance,
                    lpszClassName: SURFACE_CLASS,
                    ..zeroed()
                };
                if RegisterClassW(&class) == 0 {
                    Err(GetLastError())
                } else {
                    Ok(())
                }
            })
            .map_err(|code| {
                window_error(
                    "Die Wiedergabefenster-Klasse konnte nicht registriert werden.",
                    code,
                )
            })?;

        // Explorer's window and its child must use compatible DPI awareness.
        let parent_dpi = GetWindowDpiAwarenessContext(parent);
        if parent_dpi.is_null() {
            return Err(window_error(
                "Die Explorer-Desktop-Ebene ist nicht verfügbar.",
                GetLastError(),
            ));
        }
        let previous_dpi = SetThreadDpiAwarenessContext(parent_dpi);
        if previous_dpi.is_null() {
            return Err(window_error(
                "Der DPI-Modus des Desktop-Fensters konnte nicht übernommen werden.",
                GetLastError(),
            ));
        }
        let mut styles = WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        if layered {
            styles |= WS_EX_LAYERED;
        }
        let hwnd = CreateWindowExW(
            styles,
            SURFACE_CLASS,
            null(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            width,
            height,
            parent,
            null_mut(),
            instance,
            null(),
        );
        let creation_error = GetLastError();
        SetThreadDpiAwarenessContext(previous_dpi);
        if hwnd.is_null() {
            return Err(window_error(
                "Das Windows-Wiedergabefenster konnte nicht erstellt werden.",
                creation_error,
            ));
        }
        if layered && SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) == 0 {
            let code = GetLastError();
            DestroyWindow(hwnd);
            return Err(window_error(
                "Die Wiedergabeebene konnte nicht eingerichtet werden.",
                code,
            ));
        }
        Ok(hwnd)
    }
}

impl Surface {
    fn new(host: Host, monitor: &Monitor, video: &Playback, mpv: &Path) -> Result<Self> {
        let hwnd = create_surface_window(
            host.parent,
            !host.icons.is_null(),
            monitor.width as i32,
            monitor.height as i32,
        )?;
        let result = Self::start_player(hwnd, video, mpv);
        if result.is_err() {
            unsafe { DestroyWindow(hwnd) };
        }
        result
    }

    fn start_player(hwnd: HWND, video: &Playback, mpv: &Path) -> Result<Self> {
        let pipe = format!(r"\\.\pipe\junadesk-{}", Uuid::new_v4());
        let mut child = media::command(mpv)
            .args([
                "--no-config",
                "--no-terminal",
                "--no-audio",
                "--no-osc",
                "--no-input-default-bindings",
                "--no-input-vo-keyboard",
                "--cursor-autohide=no",
                "--stop-screensaver=no",
                "--vo=gpu",
                "--gpu-api=d3d11",
                "--hwdec=auto",
                "--loop-file=inf",
                "--panscan=1.0",
                "--no-border",
                "--autoload-files=no",
            ])
            .arg(format!("--wid={}", hwnd as usize as u32))
            .arg(format!("--input-ipc-server={pipe}"))
            .arg(if video.paused {
                "--pause=yes"
            } else {
                "--pause=no"
            })
            .arg("--")
            .arg(&video.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("mpv konnte nicht gestartet werden: {e}"))?;
        let job = unsafe { CreateJobObjectW(null(), null()) };
        let mut limit: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limit.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let managed = !job.is_null()
            && unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limit as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) != 0
                    && AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) != 0
            };
        if !managed {
            let _ = child.kill();
            let _ = child.wait();
            if !job.is_null() {
                unsafe { CloseHandle(job) };
            }
            return Err("mpv konnte nicht an den JunaDesk-Prozess gebunden werden.".into());
        }
        Ok(Self {
            hwnd,
            job,
            child,
            pipe,
            path: video.path.clone(),
            paused: video.paused,
            started: Instant::now(),
        })
    }

    fn update(&mut self, host: Host, monitor: &Monitor, video: &Playback) -> Result<()> {
        if unsafe { IsWindow(self.hwnd) == 0 } || !matches!(self.child.try_wait(), Ok(None)) {
            return Err("mpv hat die Wiedergabe unerwartet beendet.".into());
        }
        if self.paused != video.paused {
            if let Ok(mut pipe) = OpenOptions::new().write(true).open(&self.pipe) {
                if writeln!(
                    pipe,
                    "{{\"command\":[\"set_property\",\"pause\",{}]}}",
                    video.paused
                )
                .is_ok()
                {
                    self.paused = video.paused;
                }
            }
        }
        unsafe {
            let ready = !GetWindow(self.hwnd, GW_CHILD).is_null();
            if !ready {
                return if self.started.elapsed() > Duration::from_secs(12) {
                    Err("mpv konnte keine Videoausgabe erstellen.".into())
                } else {
                    Ok(())
                };
            }
            let mut point = POINT {
                x: monitor.x,
                y: monitor.y,
            };
            MapWindowPoints(null_mut(), host.parent, &mut point, 1);
            let after = if host.icons.is_null() {
                HWND_BOTTOM
            } else {
                host.icons
            };
            if SetWindowPos(
                self.hwnd,
                after,
                point.x,
                point.y,
                monitor.width as i32,
                monitor.height as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) == 0
            {
                return Err("Die Explorer-Desktop-Ebene konnte nicht aktualisiert werden.".into());
            }
        }
        Ok(())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.job) };
        let _ = self.child.kill();
        let _ = self.child.wait();
        unsafe { DestroyWindow(self.hwnd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_layered_child_in_the_parent_dpi_context() {
        unsafe {
            let original_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
            assert!(!original_dpi.is_null());
            let parent = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                null(),
                WS_POPUP,
                0,
                0,
                320,
                240,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!parent.is_null());
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            assert!(!previous.is_null());
            let child = create_surface_window(parent, true, 320, 240)
                .expect("a layered wallpaper child should be supported");
            assert_eq!(GetParent(child), parent);
            assert_ne!(GetWindowLongW(child, GWL_EXSTYLE) as u32 & WS_EX_LAYERED, 0);
            assert_ne!(
                AreDpiAwarenessContextsEqual(
                    GetWindowDpiAwarenessContext(child),
                    GetWindowDpiAwarenessContext(parent),
                ),
                0
            );
            assert_ne!(
                AreDpiAwarenessContextsEqual(
                    GetThreadDpiAwarenessContext(),
                    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                ),
                0
            );
            DestroyWindow(child);
            DestroyWindow(parent);
            SetThreadDpiAwarenessContext(original_dpi);
        }
    }
}

fn monitors() -> Vec<Monitor> {
    let identities = display_paths();
    let mut result: Vec<Monitor> = Vec::new();
    unsafe extern "system" fn callback(
        handle: HMONITOR,
        _: HDC,
        _: *mut RECT,
        arg: LPARAM,
    ) -> BOOL {
        let (identities, result) =
            &mut *(arg as *mut (&BTreeMap<String, (String, String)>, &mut Vec<Monitor>));
        let mut info: MONITORINFOEXW = zeroed();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(handle, &mut info.monitorInfo) != 0 {
            let gdi_name = string(&info.szDevice);
            let Some((id, name)) = identities.get(&gdi_name) else {
                return 1;
            };
            let rect = info.monitorInfo.rcMonitor;
            result.push(Monitor {
                id: id.clone(),
                name: name.clone(),
                width: (rect.right - rect.left) as u32,
                height: (rect.bottom - rect.top) as u32,
                x: rect.left,
                y: rect.top,
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                fullscreen: fullscreen(&rect),
            });
        }
        1
    }
    let mut data = (&identities, &mut result);
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(callback),
            &mut data as *mut _ as LPARAM,
        )
    };
    result.sort_by(|a, b| {
        b.primary
            .cmp(&a.primary)
            .then(a.x.cmp(&b.x))
            .then(a.y.cmp(&b.y))
    });
    result
}

fn display_paths() -> BTreeMap<String, (String, String)> {
    let mut result = BTreeMap::new();
    let mut paths_count = 0;
    let mut modes_count = 0;
    unsafe {
        for _ in 0..3 {
            if GetDisplayConfigBufferSizes(
                QDC_ONLY_ACTIVE_PATHS,
                &mut paths_count,
                &mut modes_count,
            ) != ERROR_SUCCESS
            {
                return result;
            }
            let mut paths = vec![zeroed::<DISPLAYCONFIG_PATH_INFO>(); paths_count as usize];
            let mut modes = vec![zeroed::<DISPLAYCONFIG_MODE_INFO>(); modes_count as usize];
            let code = QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut paths_count,
                paths.as_mut_ptr(),
                &mut modes_count,
                modes.as_mut_ptr(),
                null_mut(),
            );
            if code == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            if code != ERROR_SUCCESS {
                break;
            }
            for path in paths.iter().take(paths_count as usize) {
                let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = zeroed();
                source.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                };
                let mut target: DISPLAYCONFIG_TARGET_DEVICE_NAME = zeroed();
                target.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                };
                if DisplayConfigGetDeviceInfo(&mut source.header) != ERROR_SUCCESS as i32 {
                    continue;
                }
                let gdi_name = string(&source.viewGdiDeviceName);
                let _ = DisplayConfigGetDeviceInfo(&mut target.header);
                let mut id = string(&target.monitorDevicePath).to_ascii_lowercase();
                if id.is_empty() {
                    id = format!(
                        "adapter-{}-{}-{}",
                        path.targetInfo.adapterId.HighPart,
                        path.targetInfo.adapterId.LowPart,
                        path.targetInfo.id
                    );
                }
                let name = string(&target.monitorFriendlyDeviceName);
                let name = if name.is_empty() {
                    gdi_name.clone()
                } else {
                    name
                };
                result
                    .entry(gdi_name)
                    .and_modify(|entry: &mut (String, String)| {
                        if id < entry.0 {
                            *entry = (id.clone(), name.clone());
                        }
                    })
                    .or_insert((id, name));
            }
            break;
        }
    }
    result
}

unsafe fn fullscreen(monitor: &RECT) -> bool {
    struct Search {
        bounds: RECT,
        fullscreen: bool,
    }
    unsafe extern "system" fn test(window: HWND, data: LPARAM) -> BOOL {
        let search = &mut *(data as *mut Search);
        let mut class = [0u16; 64];
        GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        if matches!(
            string(&class).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) || IsWindowVisible(window) == 0
            || IsIconic(window) != 0
            || GetWindowLongW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0
        {
            return 1;
        }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut _ as *mut _,
            4,
        );
        if cloaked != 0 {
            return 1;
        }
        let mut rect: RECT = zeroed();
        if DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut _ as *mut _,
            size_of::<RECT>() as u32,
        ) != 0
            && GetWindowRect(window, &mut rect) == 0
        {
            return 1;
        }
        let bounds = &search.bounds;
        let overlap_x = rect.right.min(bounds.right) - rect.left.max(bounds.left);
        let overlap_y = rect.bottom.min(bounds.bottom) - rect.top.max(bounds.top);
        if overlap_x <= 0
            || overlap_y <= 0
            || i64::from(overlap_x) * i64::from(overlap_y)
                < i64::from(bounds.right - bounds.left) * i64::from(bounds.bottom - bounds.top) / 10
        {
            return 1;
        }
        search.fullscreen = rect.left <= bounds.left + 2
            && rect.top <= bounds.top + 2
            && rect.right >= bounds.right - 2
            && rect.bottom >= bounds.bottom - 2;
        0
    }
    let mut search = Search {
        bounds: *monitor,
        fullscreen: false,
    };
    EnumWindows(Some(test), &mut search as *mut _ as LPARAM);
    search.fullscreen
}
