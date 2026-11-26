//! Native Windows notification-area (system tray) integration.
//!
//! `start` creates a private hidden popup window on a dedicated thread. The
//! UI thread only receives [`TrayAction`] values from the returned channel; it
//! is never called from the Win32 window procedure.
//!
//! Example:
//! ```no_run
//! let tray = crate::tray::start(None);
//! while let Ok(action) = tray.rx.try_recv() {
//!     match action {
//!         crate::tray::TrayAction::Open => { /* show/activate the UI window */ }
//!         crate::tray::TrayAction::MoveTo(_) => { /* move the UI window */ }
//!         crate::tray::TrayAction::Exit => { /* request application shutdown */ }
//!     }
//! }
//! ```
//!
//! Limitations: this module is Windows-only and intentionally does not own or
//! manipulate the WinUI window. `start` has no fallible return type, so a
//! native setup failure leaves `rx` disconnected. The caller must poll or
//! bridge `rx`; this module does not run WinUI callbacks from the tray thread.

#![cfg(windows)]

use std::mem::size_of;
use std::ptr::null;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use window_lock_master_rust::placement::{monitor_name, monitors, MonitorDevice};
use windows::Win32::System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE,
    NIF_TIP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, LoadIconW, LoadImageW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    SetWindowLongPtrW, TrackPopupMenu, TranslateMessage, UnregisterClassW, CS_DBLCLKS,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDI_APPLICATION, IMAGE_ICON, LR_SHARED,
    MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MONITORINFOF_PRIMARY, MSG,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, WNDCLASSW, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK,
    WM_NCCREATE, WM_RBUTTONUP, WM_USER, WS_EX_TOOLWINDOW, WS_POPUP,
};

const CALLBACK_MESSAGE: u32 = WM_USER + 1;
const SHUTDOWN_MESSAGE: u32 = WM_USER + 2;
const OPEN_COMMAND: usize = 1;
const EXIT_COMMAND: usize = 2;
const FIRST_MONITOR_COMMAND: usize = 100;
const TRAY_ID: u32 = 1;

const CLASS_NAME: &[u16] = &[
    'W' as u16, 'i' as u16, 'n' as u16, 'd' as u16, 'o' as u16, 'w' as u16,
    'L' as u16, 'o' as u16, 'c' as u16, 'k' as u16, 'M' as u16, 'a' as u16,
    's' as u16, 't' as u16, 'e' as u16, 'r' as u16, 'T' as u16, 'r' as u16,
    'a' as u16, 'y' as u16, 0,
];
const TOOLTIP: &[u16] = &[
    '窗' as u16, '口' as u16, '管' as u16, '理' as u16, '器' as u16, ' ' as u16,
    'W' as u16, 'i' as u16, 'n' as u16, 'd' as u16, 'o' as u16, 'w' as u16,
    'L' as u16, 'o' as u16, 'c' as u16, 'k' as u16, 'M' as u16, 'a' as u16,
    's' as u16, 't' as u16, 'e' as u16, 'r' as u16, ' ' as u16, 'R' as u16,
    'u' as u16, 's' as u16, 't' as u16, 0,
];
const OPEN_TEXT: &[u16] = &['打' as u16, '开' as u16, 0];
const EXIT_TEXT: &[u16] = &['退' as u16, '出' as u16, 0];
const TASKBAR_CREATED: &[u16] = &[
    'T' as u16, 'a' as u16, 's' as u16, 'k' as u16, 'b' as u16, 'a' as u16,
    'r' as u16, 'C' as u16, 'r' as u16, 'e' as u16, 'a' as u16, 't' as u16,
    'e' as u16, 'd' as u16, 0,
];

/// Actions emitted by the tray thread. They are independent of WinUI types.
pub enum TrayAction {
    Open,
    MoveTo(MonitorDevice),
    Exit,
}

/// Owns the tray thread and its event receiver.
///
/// Dropping this value posts a private shutdown message and joins the thread;
/// cleanup therefore happens on the tray thread that owns the HWND, HICON, and
/// notification-area registration.
pub struct TrayHandle {
    pub rx: Receiver<TrayAction>,
    shutdown: Option<isize>,
    pub hwnd: Option<isize>,
    pub owner_pid: u32,
    thread: Option<JoinHandle<()>>,
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        if let Some(hwnd) = self.shutdown.take() {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), SHUTDOWN_MESSAGE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts the dedicated tray message-loop thread.
pub fn start(main_window: Option<HWND>) -> TrayHandle {
    let main_window = main_window.map(|hwnd| hwnd.0 as isize);
    let (action_tx, action_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("window-lock-master-tray".to_owned())
        .spawn(move || tray_thread(action_tx, ready_tx, main_window))
        .expect("failed to start tray thread");

    let shutdown = ready_rx.recv().ok().flatten();
    let owner_pid = unsafe { GetCurrentProcessId() };
    TrayHandle { rx: action_rx, shutdown, hwnd: shutdown, owner_pid, thread: Some(thread) }
}

struct TrayState {
    actions: Sender<TrayAction>,
    main_window: Option<isize>,
    taskbar_created: u32,
    icon: Option<windows::Win32::UI::WindowsAndMessaging::HICON>,
}

fn tray_thread(actions: Sender<TrayAction>, ready: mpsc::SyncSender<Option<isize>>, main_window: Option<isize>) {
    unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(module) => module.into(),
            Err(_) => { let _ = ready.send(None); return; }
        };
        let taskbar_created = RegisterWindowMessageW(PCWSTR(TASKBAR_CREATED.as_ptr()));
        let class = WNDCLASSW {
            style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(CLASS_NAME.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            let _ = ready.send(None);
            return;
        }

        // This state remains on this thread until the message loop exits.
        let mut state = TrayState { actions, main_window, taskbar_created, icon: None };
        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(CLASS_NAME.as_ptr()),
            PCWSTR(CLASS_NAME.as_ptr()),
            WS_POPUP,
            0, 0, 0, 0, None, None, Some(instance),
            Some(&mut state as *mut TrayState as *const _),
        ) {
            Ok(window) => window,
            Err(_) => {
                let _ = UnregisterClassW(PCWSTR(CLASS_NAME.as_ptr()), Some(instance));
                let _ = ready.send(None);
                return;
            }
        };

        // Retry is driven by Explorer's registered TaskbarCreated broadcast.
        add_icon(hwnd, &mut state);
        let _ = ready.send(Some(hwnd.0 as isize));

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if !hwnd.is_invalid() { let _ = DestroyWindow(hwnd); }
        remove_icon(hwnd, &mut state);
        let _ = UnregisterClassW(PCWSTR(CLASS_NAME.as_ptr()), Some(instance));
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam.0
            as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
        if !create.is_null() {
            let state = (*create).lpCreateParams as *mut TrayState;
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayState;
    if state_ptr.is_null() { return DefWindowProcW(hwnd, message, wparam, lparam); }
    // TrackPopupMenu can re-enter the window procedure. Drop all state borrows first.
    if message == CALLBACK_MESSAGE && lparam.0 as u32 == WM_RBUTTONUP {
        let actions = (*state_ptr).actions.clone();
        let main_window = (*state_ptr).main_window;
        if let Some(action) = show_menu(hwnd, main_window) {
            let exit = matches!(action, TrayAction::Exit);
            let _ = actions.send(action);
            if exit { let _ = DestroyWindow(hwnd); }
        }
        return LRESULT(0);
    }
    let state = &mut *state_ptr;

    if state.taskbar_created != 0 && message == state.taskbar_created {
        add_icon(hwnd, state);
        return LRESULT(0);
    }
    match message {
        CALLBACK_MESSAGE => {
            match lparam.0 as u32 {
                WM_LBUTTONDBLCLK => send_action(state, TrayAction::Open),

                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as usize {
                OPEN_COMMAND => send_action(state, TrayAction::Open),
                EXIT_COMMAND => {
                    send_action(state, TrayAction::Exit);
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        SHUTDOWN_MESSAGE => { let _ = DestroyWindow(hwnd); LRESULT(0) }
        WM_DESTROY => {
            remove_icon(hwnd, state);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn send_action(state: &TrayState, action: TrayAction) { let _ = state.actions.send(action); }

unsafe fn show_menu(hwnd: HWND, main_window: Option<isize>) -> Option<TrayAction> {
    let menu = CreatePopupMenu().ok()?;
    let _ = AppendMenuW(menu, MF_STRING, OPEN_COMMAND, PCWSTR(OPEN_TEXT.as_ptr()));
    let mut targets = Vec::new();
    if let Some(main) = main_window {
        let current = MonitorFromWindow(HWND(main as *mut _), MONITOR_DEFAULTTONEAREST);
        let mut displays = monitors();
        displays.sort_by_key(|(_, info)| info.szDevice);
        let title = windows_core::w!("移动本程序窗口到显示器");
        if let Ok(submenu) = CreatePopupMenu() {
            for (monitor, info) in displays {
                let device = monitor_name(&info.szDevice);
                let name = device.strip_prefix("\\\\.\\").unwrap_or(&device);
                let bounds = info.monitorInfo.rcMonitor;
                let primary = if info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0 { " · 主屏" } else { "" };
                let here = if monitor == current { " · 当前" } else { "" };
                let text = windows_core::HSTRING::from(format!(
                    "{name}  {}×{}{primary}{here}", bounds.right - bounds.left, bounds.bottom - bounds.top,
                ));
                let flags = if monitor == current { MF_STRING | MF_CHECKED } else { MF_STRING };
                if AppendMenuW(submenu, flags, FIRST_MONITOR_COMMAND + targets.len(), PCWSTR(text.as_ptr())).is_ok() {
                    targets.push(info.szDevice);
                }
            }
            if targets.is_empty() || AppendMenuW(menu, MF_POPUP, submenu.0 as usize, title).is_err() {
                let _ = DestroyMenu(submenu);
                targets.clear();
                let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, title);
            }
        }
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR(null()));
    let _ = AppendMenuW(menu, MF_STRING, EXIT_COMMAND, PCWSTR(EXIT_TEXT.as_ptr()));
    let mut point = POINT::default();
    let command = if GetCursorPos(&mut point).is_ok() {
        let _ = SetForegroundWindow(hwnd);
        let selected = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x, point.y, Some(0), hwnd, None).0 as usize;
        // Required by TrackPopupMenu for reliable dismissal on Windows shells.
        let _ = PostMessageW(Some(hwnd), 0, WPARAM(0), LPARAM(0));
        selected
    } else { 0 };
    let _ = DestroyMenu(menu);
    match command {
        OPEN_COMMAND => Some(TrayAction::Open),
        EXIT_COMMAND => Some(TrayAction::Exit),
        command => command.checked_sub(FIRST_MONITOR_COMMAND)
            .and_then(|index| targets.get(index).copied()).map(TrayAction::MoveTo),
    }
}

unsafe fn add_icon(hwnd: HWND, state: &mut TrayState) {
    if state.icon.is_none() {
        state.icon = GetModuleHandleW(None).ok().and_then(|module| {
            LoadImageW(Some(module.into()), PCWSTR(101usize as *const u16), IMAGE_ICON, 16, 16, LR_SHARED)
                .ok().map(|handle| windows::Win32::UI::WindowsAndMessaging::HICON(handle.0))
        }).or_else(|| LoadIconW(None, IDI_APPLICATION).ok());
    }
    let Some(icon) = state.icon else { return };
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: CALLBACK_MESSAGE,
        hIcon: icon,
        ..Default::default()
    };
    data.szTip[..TOOLTIP.len()].copy_from_slice(TOOLTIP);
    let _ = Shell_NotifyIconW(NIM_ADD, &data);
}

unsafe fn remove_icon(hwnd: HWND, state: &mut TrayState) {
    let Some(icon) = state.icon.take() else { return };
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        hIcon: icon,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    // Both application and fallback icons are shared handles.
}