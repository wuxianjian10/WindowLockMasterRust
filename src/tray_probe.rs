mod tray;
use std::time::Duration;
use windows::Win32::Foundation::{LPARAM,WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW,PostMessageW,WM_USER,WM_LBUTTONDBLCLK,WM_COMMAND};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows_core::w;
fn main(){
 let tray=tray::start(None);
 let hwnd=tray.hwnd.map(|v|windows::Win32::Foundation::HWND(v as *mut _)).expect("tray HWND");
 unsafe{PostMessageW(Some(hwnd),WM_USER+1,WPARAM(1),LPARAM(WM_LBUTTONDBLCLK as isize)).unwrap();}
 assert!(matches!(tray.rx.recv_timeout(Duration::from_secs(2)).unwrap(),tray::TrayAction::Open));
 unsafe{PostMessageW(Some(hwnd),WM_COMMAND,WPARAM(2),LPARAM(0)).unwrap();}
 assert!(matches!(tray.rx.recv_timeout(Duration::from_secs(2)).unwrap(),tray::TrayAction::Exit));
 drop(tray);
 let live=unsafe{FindWindowW(w!("WindowLockMasterTray"),None)};
 let mut pid=0; if let Ok(other)=live {unsafe{windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(other,Some(&mut pid));}}
 assert!(live.is_err() || pid!=GetCurrentProcessId(),"only this probe's HWND must be destroyed");
 println!("PASS: native tray double-click message -> Open; Exit -> thread joined and HWND destroyed (synthetic message test, not physical mouse)");
}
