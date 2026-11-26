use std::{path::PathBuf,thread,time::{Duration,Instant},io::{BufRead,Write}};
use window_lock_master_rust::backend::{self,Command};
use windows::Win32::{Foundation::{HWND,RECT},Graphics::Gdi::{MonitorFromWindow,MONITOR_DEFAULTTONEAREST},UI::WindowsAndMessaging::*};
use windows_core::w;
fn pump(ms:u64){let until=Instant::now()+Duration::from_millis(ms);while Instant::now()<until{unsafe{let mut msg=MSG::default();while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool(){let _=TranslateMessage(&msg);DispatchMessageW(&msg);}}thread::sleep(Duration::from_millis(10));}}
fn mon(hwnd:HWND)->isize{unsafe{MonitorFromWindow(hwnd,MONITOR_DEFAULTTONEAREST).0 as isize}}
fn main(){unsafe{
 if std::env::args().any(|a|a=="--fixture"){
  let h=CreateWindowExW(WINDOW_EX_STYLE(0),w!("STATIC"),w!("WLM OWNED MOVEMENT TEST"),WS_OVERLAPPEDWINDOW|WS_VISIBLE,100,100,500,320,None,None,None,None).unwrap();
  println!("{}",h.0 as isize);std::io::stdout().flush().unwrap();
  while IsWindow(Some(h)).as_bool(){pump(25);}return;
 }
 let data=PathBuf::from(format!("K:/HermesProjects/WindowLockMasterRust/artifacts/owned-move-test-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()));let b=backend::start(data.clone(),false);pump(400);
 let monitors=b.snapshot.lock().unwrap().monitors.clone();assert!(monitors.len()>=2,"two real monitors required");
 let mut child=std::process::Command::new(std::env::current_exe().unwrap()).arg("--fixture").stdout(std::process::Stdio::piped()).spawn().unwrap();
 let mut line=String::new();std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
 let hwnd=HWND(line.trim().parse::<isize>().unwrap() as *mut _);
 let id=hwnd.0 as isize;
 b.tx.send(Command::Move(vec![id],monitors[0].id.clone())).unwrap();pump(900);let first=mon(hwnd);
 b.tx.send(Command::Move(vec![id],monitors[1].id.clone())).unwrap();pump(1000);let second=mon(hwnd);assert_ne!(first,second,"ordinary move must change physical monitor");
 let _=ShowWindow(hwnd,SW_MAXIMIZE);pump(300);let before=mon(hwnd);b.tx.send(Command::Move(vec![id],monitors[1].id.clone())).unwrap();pump(500);assert!(IsZoomed(hwnd).as_bool());assert_eq!(before,mon(hwnd));
 b.tx.send(Command::Move(vec![id],monitors[0].id.clone())).unwrap();pump(1200);assert!(IsZoomed(hwnd).as_bool(),"maximized state preserved");assert_eq!(first,mon(hwnd));
 let _=ShowWindow(hwnd,SW_RESTORE);pump(200);let _=ShowWindow(hwnd,SW_MINIMIZE);pump(200);
 b.tx.send(Command::Move(vec![id],monitors[1].id.clone())).unwrap();pump(1400);assert!(IsIconic(hwnd).as_bool(),"minimized state preserved");let _=ShowWindow(hwnd,SW_RESTORE);pump(500);assert_eq!(second,mon(hwnd),"restored minimized window truly on destination");
 b.tx.send(Command::Lock(vec![id])).unwrap();pump(300);b.tx.send(Command::Move(vec![id],monitors[0].id.clone())).unwrap();pump(1400);assert_eq!(first,mon(hwnd));pump(1000);assert_eq!(first,mon(hwnd),"lock must not pull back to stale target");
 b.tx.send(Command::Unlock(vec![id])).unwrap();pump(100);
 // Passive lock leaves minimized windows untouched, then corrects a restored
 // window after external movement without needing a drag-end event.
 b.tx.send(Command::Lock(vec![id])).unwrap();pump(250);let _=ShowWindow(hwnd,SW_MINIMIZE);pump(800);assert!(IsIconic(hwnd).as_bool());
 let _=ShowWindow(hwnd,SW_RESTORE);pump(150);let _=SetWindowPos(hwnd,None,2800,700,0,0,SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE);pump(1400);assert_eq!(first,mon(hwnd),"external move must be corrected once");
 b.tx.send(Command::Unlock(vec![id])).unwrap();pump(150);
 b.tx.send(Command::AddBlacklist("WindowLockMasterOwnedMoveTest.exe".into())).unwrap();pump(250);b.tx.send(Command::Move(vec![id],monitors[1].id.clone())).unwrap();pump(500);assert_eq!(first,mon(hwnd),"blacklist forbids movement");
 b.tx.send(Command::Shutdown).unwrap();pump(200);PostMessageW(Some(hwnd),WM_CLOSE,Default::default(),Default::default()).unwrap();child.wait().unwrap();
 let text=std::fs::read_to_string(data.join("backend.log")).unwrap();println!("{text}");assert!(!text.contains("phase=Failed"),"fixture task failure");println!("PASS owned real-window movement: normal cross-monitor, already-target maximized, maximized cross-monitor, minimized restore destination, lock binding stable");
}}
