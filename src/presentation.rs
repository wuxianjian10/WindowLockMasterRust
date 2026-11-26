use naui_winui3::{IWindowNative, Microsoft::UI::Xaml::{Markup::XamlReader, Media::SystemBackdrop}};
use naui_windows::Window;
use windows::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
    Graphics::Gdi::{MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONEAREST},
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};
use windows_core::{HSTRING, Interface, PCWSTR, Result};
use window_lock_master_rust::placement::{find_monitor, fit_frame, monitor_info, MonitorDevice};

const ICON_ID: usize = 101;

pub fn load_icon(size: i32) -> Result<HICON> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        LoadImageW(Some(module.into()), PCWSTR(ICON_ID as *const u16), IMAGE_ICON, size, size, LR_SHARED)
            .map(|handle| HICON(handle.0))
    }
}

pub fn apply_icon(hwnd: HWND) -> Result<()> {
    for (kind, size) in [(ICON_SMALL, 16), (ICON_BIG, 32)] {
        let icon = load_icon(size)?;
        unsafe { SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(icon.0 as isize))); }
    }
    Ok(())
}

pub fn apply_glass(window: &Window) -> Result<HSTRING> {
    let backdrop: SystemBackdrop = XamlReader::Load(&HSTRING::from(
        r#"<DesktopAcrylicBackdrop xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"/>"#,
    ))?.cast()?;
    let class = backdrop.cast::<windows_core::IInspectable>()?.GetRuntimeClassName()?;
    window.native_window().SetSystemBackdrop(&backdrop)?;
    Ok(class)
}


pub struct Placement {
    pub hwnd: HWND,
    monitor: HMONITOR,
    device: MonitorDevice,
    work: RECT,
    dpi: u32,
    last_frame: RECT,
    test_limit: Option<(i32, i32)>,
    startup_until: std::time::Instant,
}

impl Placement {
    pub fn new(window: &Window, diagnostic: bool) -> Result<Self> {
        let hwnd = unsafe { window.native_window().cast::<IWindowNative>()?.WindowHandle()? };
        let mut point = POINT::default();
        let cursor = unsafe { GetCursorPos(&mut point) }.is_ok();
        let mut limit = None;
        // Read-only GUI tests can emulate a small work area on a real monitor.
        if diagnostic {
            if let Ok(value) = std::env::var("WLM_TEST_WORK") {
                let fields: Vec<_> = value.split(',').filter_map(|v| v.parse::<i32>().ok()).collect();
                if let [x, y, width, height] = fields[..] {
                    if width > 0 && height > 0 {
                        point = POINT { x, y };
                        limit = Some((width, height));
                    }
                }
            }
        }
        let monitor = unsafe {
            if cursor || limit.is_some() { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) }
            else { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
        };
        let info = monitor_info(monitor).ok_or_else(|| {
            windows_core::Error::new(windows::Win32::Foundation::E_FAIL, "无法读取显示器工作区")
        })?;
        let work = Self::limited(info.monitorInfo.rcWork, limit);
        let mut placement = Self {
            hwnd, monitor, device: info.szDevice, work, dpi: 0, last_frame: RECT::default(), test_limit: limit,
            startup_until: std::time::Instant::now() + std::time::Duration::from_secs(1),
        };
        // Move and size together while hidden. A large initial frame can otherwise
        // select the neighbouring monitor before WinUI runs its first layout.
        placement.set_frame(fit_frame(RECT { right: 900, bottom: 660, ..Default::default() }, work, true))?;
        placement.dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let scale = f64::from(placement.dpi) / 96.0;
        let width = f64::from(work.right - work.left) / scale;
        let height = f64::from(work.bottom - work.top) / scale;
        window.set_size((width - 32.0).clamp(1.0, 1080.0), (height - 65.0).clamp(1.0, 790.0));
        Ok(placement)
    }

    fn limited(mut work: RECT, limit: Option<(i32, i32)>) -> RECT {
        if let Some((width, height)) = limit {
            work.right = work.right.min(work.left + width);
            work.bottom = work.bottom.min(work.top + height);
        }
        work
    }

    pub fn fit(&mut self, center: bool) -> Result<()> {
        let frame = self.frame()?;
        let target = fit_frame(frame, self.work, center);
        if target != frame { self.set_frame(target)?; }
        self.last_frame = self.frame()?;
        Ok(())
    }

    pub fn move_to(&mut self, window: &Window, device: MonitorDevice) -> Result<()> {
        let (monitor, info) = find_monitor(&device).ok_or_else(|| {
            windows_core::Error::new(windows::Win32::Foundation::E_FAIL, "目标显示器已断开")
        })?;
        let current = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) };
        if current == monitor {
            if unsafe { IsIconic(self.hwnd) }.as_bool() {
                window.set_state(naui_core::WindowState::Normal);
            }
            window.show();
            return self.update(true);
        }

        let work = Self::limited(info.monitorInfo.rcWork, self.test_limit);
        let maximized = unsafe { IsZoomed(self.hwnd) }.as_bool();
        let minimized = unsafe { IsIconic(self.hwnd) }.as_bool();
        if maximized || minimized {
            window.set_state(naui_core::WindowState::Normal);
        }
        let frame = if minimized || maximized { self.last_frame } else { self.frame()? };
        self.set_frame(fit_frame(frame, work, true))?;
        // A per-monitor DPI transition can change the outer rectangle during SetWindowPos.
        let target = fit_frame(self.frame()?, work, true);
        if target != self.frame()? { self.set_frame(target)?; }
        if unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) } != monitor {
            return Err(windows_core::Error::new(windows::Win32::Foundation::E_FAIL, "主窗口未到达目标显示器"));
        }
        self.monitor = monitor;
        self.device = device;
        self.work = work;
        self.dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
        self.last_frame = self.frame()?;
        window.show();
        if maximized { window.set_state(naui_core::WindowState::Maximized); }
        Ok(())
    }

    pub fn update(&mut self, reopening: bool) -> Result<()> {
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() || IsIconic(self.hwnd).as_bool() || IsZoomed(self.hwnd).as_bool() {
                return Ok(());
            }
            let mut gui = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
            if GetGUIThreadInfo(GetWindowThreadProcessId(self.hwnd, None), &mut gui).is_ok()
                && gui.flags.contains(GUI_INMOVESIZE) {
                return Ok(());
            }
        }
        self.sync_monitor(reopening)
    }

    fn sync_monitor(&mut self, reopening: bool) -> Result<()> {
        let anchored_info = monitor_info(self.monitor).filter(|info| info.szDevice == self.device);
        let mut handle_changed = false;
        let anchored_info = anchored_info.or_else(|| {
            let (monitor, info) = find_monitor(&self.device)?;
            self.monitor = monitor;
            handle_changed = true;
            Some(info)
        });
        let anchored_work = anchored_info.map(|info| Self::limited(info.monitorInfo.rcWork, self.test_limit));
        let current = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) };
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
        if let Some(work) = anchored_work {
            if work != self.work || ((reopening || handle_changed) && current != self.monitor) {
                self.work = work;
                let frame = self.frame()?;
                let target = fit_frame(self.last_frame, work, false);
                if frame != target { self.set_frame(target)?; }
                self.dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
                return self.fit(false);
            }
        }
        if anchored_work.is_some() && std::time::Instant::now() < self.startup_until {
            return self.fit(false);
        }
        let mut geometry_changed = dpi != self.dpi;
        if current != self.monitor || anchored_work.is_none() {
            if let Some(info) = monitor_info(current) {
                self.monitor = current;
                self.device = info.szDevice;
                self.work = Self::limited(info.monitorInfo.rcWork, self.test_limit);
                geometry_changed = true;
            }
        }
        self.dpi = dpi;
        if geometry_changed || reopening {
            self.fit(false)
        } else {
            self.last_frame = self.frame()?;
            Ok(())
        }
    }

    fn frame(&self) -> Result<RECT> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut rect)?; }
        Ok(rect)
    }

    fn set_frame(&mut self, rect: RECT) -> Result<()> {
        unsafe {
            SetWindowPos(self.hwnd, None, rect.left, rect.top, rect.right - rect.left,
                rect.bottom - rect.top, SWP_NOACTIVATE | SWP_NOZORDER)
        }
    }
}
