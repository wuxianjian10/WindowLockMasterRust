use std::{sync::{mpsc,Arc,Mutex},thread,time::Duration};
use windows::Win32::{Foundation::{HWND,LPARAM,WPARAM},System::Threading::GetCurrentThreadId,UI::{Input::KeyboardAndMouse::{RegisterHotKey,UnregisterHotKey,MOD_ALT,MOD_CONTROL,MOD_SHIFT,MOD_NOREPEAT},WindowsAndMessaging::*}};
use crate::backend::{Command,Snapshot};
pub struct Hotkeys{thread:u32,join:Option<thread::JoinHandle<()>>}
impl Drop for Hotkeys{fn drop(&mut self){unsafe{let _=PostThreadMessageW(self.thread,WM_QUIT,WPARAM(0),LPARAM(0));}if let Some(t)=self.join.take(){let _=t.join();}}}
pub fn start(tx:mpsc::Sender<Command>,snapshot:Arc<Mutex<Snapshot>>,data:std::path::PathBuf)->Hotkeys{
 let (ready,rx)=mpsc::channel();
 let join=thread::spawn(move||unsafe{
  let mut msg=MSG::default();let _=PeekMessageW(&mut msg,None,0,0,PM_NOREMOVE);
  let keys=[(1,0x25,false),(2,0x27,false),(3,0x4c,false),(4,0x55,false),(5,0x4d,false),(6,0x4d,true)];let mut registered=vec![];let mut failures=vec![];
  for (id,key,shift) in keys{let mods=MOD_ALT|MOD_CONTROL|MOD_NOREPEAT|if shift{MOD_SHIFT}else{Default::default()};match RegisterHotKey(None,id,mods,key){Ok(())=>registered.push(id),Err(e)=>failures.push(format!("快捷键 {id}: {e}"))}}
  let _=std::fs::write(data.join("hotkeys.log"),format!("registered={}/6\n{}",registered.len(),failures.join("\n")));let _=ready.send(GetCurrentThreadId());
  while GetMessageW(&mut msg,None,0,0).0>0{
   if msg.message!=WM_HOTKEY{continue;}
   let id=msg.wParam.0 as i32;let s=snapshot.lock().unwrap_or_else(|e|e.into_inner()).clone();let foreground=GetForegroundWindow();let hwnd=foreground.0 as isize;
   let command=match id{
    3=>Some(Command::Lock(vec![hwnd])),4=>Some(Command::Unlock(vec![hwnd])),
    6=>Some(Command::MouseLock(None)),
    5=>if s.mouse_target.is_some(){Some(Command::MouseLock(None))}else{let mut point=Default::default();let _=GetCursorPos(&mut point);let mon=windows::Win32::Graphics::Gdi::MonitorFromPoint(point,windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST);let mut info=windows::Win32::Graphics::Gdi::MONITORINFOEXW::default();info.monitorInfo.cbSize=std::mem::size_of_val(&info) as u32;let _=windows::Win32::Graphics::Gdi::GetMonitorInfoW(mon,&mut info.monitorInfo);let n=info.szDevice.iter().position(|v|*v==0).unwrap_or(info.szDevice.len());Some(Command::MouseLock(Some(String::from_utf16_lossy(&info.szDevice[..n]))))},
    1|2=>{let row=s.windows.iter().find(|w|w.id==hwnd);let current=row.and_then(|r|s.monitors.iter().position(|m|m.id==r.monitor));current.and_then(|i|{let n=s.monitors.len();if n<2{None}else{let j=if id==1{(i+n-1)%n}else{(i+1)%n};Some(Command::Move(vec![hwnd],s.monitors[j].id.clone()))}})},_=>None};
   if let Some(c)=command{let _=tx.send(c);}
  }
  for id in registered{let _=UnregisterHotKey(None,id);}
 });
 Hotkeys{thread:rx.recv_timeout(Duration::from_secs(3)).unwrap_or(0),join:Some(join)}
}
