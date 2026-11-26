#![windows_subsystem = "windows"]
use std::{path::PathBuf, sync::OnceLock};
use naui_core::{ListItem, Orientation, Padding, Settings};
use windows_core::{Interface, PCWSTR};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetModuleFileNameW};

static LOG: OnceLock<PathBuf> = OnceLock::new();
fn record(text: &str) {
    use std::io::Write;
    let path = LOG.get_or_init(|| {
        let dir = std::env::var_os("WLM_PROBE_DATA").map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_exe().unwrap().parent().unwrap().join("probe-data"));
        let _ = std::fs::create_dir_all(&dir); dir.join("probe.log")
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) { let _ = writeln!(f, "{text}"); }
}
fn module(name: &str) {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        match GetModuleHandleW(PCWSTR(wide.as_ptr())) {
            Ok(h) => { let mut buf = [0u16; 4096]; let n = GetModuleFileNameW(Some(h), &mut buf); record(&format!("MODULE {}={}", name, String::from_utf16_lossy(&buf[..n as usize]))); }
            Err(e) => record(&format!("MODULE {name} unavailable: {e}")),
        }
    }
}
fn main() {
    record("START Rust genuine WinUI probe");
    let result = naui_windows::run(Settings::new("WindowLockMasterRustProbe"), |ui| {
        let window = ui.window("WindowLockMaster Rust · WinUI 3 技术验证", 960.0, 640.0)?;
        let pane = ui.stack(Orientation::Vertical)?;
        pane.set_spacing(16.0); pane.set_padding(Padding::all(24.0));
        let title = ui.label("窗口管理 · Rust / WinUI 3")?;
        let status = ui.label("这是界面与便携部署探针，尚未启用窗口移动与锁定。")?;
        let list = ui.list()?;
        list.set_items(&[ListItem::new("普通窗口").detail("待接入实时状态"), ListItem::new("最小化窗口").detail("被动监控不还原"), ListItem::new("最大化窗口").detail("保留状态，目标正确时不操作")]);
        record(&format!("CLASS window={}", window.native_window().cast::<windows_core::IInspectable>().unwrap().GetRuntimeClassName().unwrap_or_default()));
        record(&format!("CLASS list={}", list.native_list_view().cast::<windows_core::IInspectable>().unwrap().GetRuntimeClassName().unwrap_or_default()));
        let button = ui.button("验证按钮事件")?;
        button.on_click({ let status = status.clone(); move || { status.set_text("按钮事件已到达 Rust UI 线程"); record("BUTTON_CLICK verified"); } });
        pane.append(&title); pane.append(&status); pane.append(&list); pane.append(&button);
        window.set_child(&pane); window.show();
        record("WINDOW shown");
        module("Microsoft.UI.Xaml.dll"); module("Microsoft.WindowsAppRuntime.dll");
        if std::env::args().any(|a| a == "--smoke") {
            let ui = ui.clone();
            let close = ui.clone().tasks().channel(move |()| { record("SMOKE_EXIT"); ui.quit(); });
            std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_secs(4)); let _ = close.send(()); });
        }
        Ok(())
    });
    match result { Ok(()) => record("EXIT success"), Err(e) => { record(&format!("ERROR {e}")); std::process::exit(1); } }
}
