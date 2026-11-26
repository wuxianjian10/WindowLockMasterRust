use windows::Win32::{
    Foundation::{LPARAM, RECT},
    Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW},
};
use windows_core::BOOL;

pub type MonitorDevice = [u16; 32];

pub fn monitor_info(monitor: HMONITOR) -> Option<MONITORINFOEXW> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of_val(&info) as u32;
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool();
    let work = info.monitorInfo.rcWork;
    (ok && work.right > work.left && work.bottom > work.top).then_some(info)
}

pub fn monitors() -> Vec<(HMONITOR, MONITORINFOEXW)> {
    type Entry = (HMONITOR, MONITORINFOEXW);
    unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let entries = &mut *(data.0 as *mut Vec<Entry>);
        if let Some(info) = monitor_info(monitor) {
            entries.push((monitor, info));
        }
        true.into()
    }
    let mut entries: Vec<Entry> = Vec::new();
    unsafe { let _ = EnumDisplayMonitors(None, None, Some(visit), LPARAM(&mut entries as *mut _ as isize)); }
    entries
}

pub fn find_monitor(device: &MonitorDevice) -> Option<(HMONITOR, MONITORINFOEXW)> {
    monitors().into_iter().find(|(_, info)| &info.szDevice == device)
}

pub fn monitor_name(device: &MonitorDevice) -> String {
    let length = device.iter().position(|&value| value == 0).unwrap_or(device.len());
    String::from_utf16_lossy(&device[..length])
}

/// Fit the outer frame in physical pixels, preserving its position if possible.
pub fn fit_frame(frame: RECT, work: RECT, center: bool) -> RECT {
    let work_width = (work.right - work.left).max(1);
    let work_height = (work.bottom - work.top).max(1);
    let width = (frame.right - frame.left).clamp(1, work_width);
    let height = (frame.bottom - frame.top).clamp(1, work_height);
    let left = if center { work.left + (work_width - width) / 2 } else { frame.left }
        .clamp(work.left, work.left + work_width - width);
    let top = if center { work.top + (work_height - height) / 2 } else { frame.top }
        .clamp(work.top, work.top + work_height - height);
    RECT { left, top, right: left + width, bottom: top + height }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn small_secondary_screen_contains_oversized_frame() {
        let work = rect(-800, 40, 0, 540);
        assert_eq!(fit_frame(rect(0, 0, 1200, 900), work, true), work);
    }

    #[test]
    fn recovery_keeps_frame_on_original_screen() {
        let work = rect(2560, 0, 3360, 560);
        assert_eq!(fit_frame(rect(0, 90, 660, 760), work, false), rect(2560, 0, 3220, 560));
    }

    #[test]
    fn ordinary_user_position_is_preserved() {
        let work = rect(0, 0, 2048, 1104);
        let frame = rect(300, 110, 1200, 800);
        assert_eq!(fit_frame(frame, work, false), frame);
    }
}
