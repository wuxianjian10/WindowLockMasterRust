#![windows_subsystem = "windows"]
mod backend;
mod tray;
mod hotkeys;
mod presentation;
mod ui;
use std::path::PathBuf;
fn main() {
    let isolated_mode = std::env::args().any(|arg| arg == "--smoke" || arg == "--readonly");
    let instance = if isolated_mode {
        None
    } else {
        let handle = unsafe { windows::Win32::System::Threading::CreateMutexW(None,false,windows_core::w!("Local\\WindowLockMasterRust2")) };
        let Ok(handle) = handle else { return; };
        if unsafe {windows::Win32::Foundation::GetLastError()} == windows::Win32::Foundation::ERROR_ALREADY_EXISTS {return;}
        Some(handle)
    };
    let mut args = std::env::args().skip(1);
    let mut readonly = false;
    let mut smoke = false;
    let mut data = None;
    while let Some(arg) = args.next() {
        match arg.as_str() { "--readonly" => readonly=true, "--smoke" => {readonly=true;smoke=true}, "--data-dir" => data=args.next().map(PathBuf::from), _=>{} }
    }
    let data = data.unwrap_or_else(|| PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("WindowLockMasterRust"));
    let _ = std::fs::create_dir_all(&data);
    let log=data.join("ui.log");
    let backend=backend::start(data.clone(),readonly);
    let tx=backend.tx.clone();
    let termination=backend.termination.clone();
    let hotkeys=if readonly{None}else{Some(hotkeys::start(tx.clone(),backend.snapshot.clone(),data.clone()))};
    let result=naui_windows::run(naui_core::Settings::new("WindowLockMasterRust"), move |ui| {
        ui::build(ui,backend,smoke,readonly,&data).map_err(|e|naui_core::Error::new("界面初始化",e.to_string()))
    });
    drop(hotkeys);
    ui::clear_handlers();
    let _=tx.send(backend::Command::Shutdown);
    if let Some(worker)=termination.lock().unwrap_or_else(|e|e.into_inner()).take(){let _=worker.join();}
    if let Err(e)=result { let _=std::fs::write(log,format!("ERROR {e}\n")); }
    if let Some(instance) = instance {
        unsafe { let _ = windows::Win32::Foundation::CloseHandle(instance); }
    }
}
