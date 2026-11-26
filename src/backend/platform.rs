use super::*;
#[path="rules.rs"] mod rules;
use std::{collections::{HashMap, HashSet}, ffi::OsString, os::windows::ffi::{OsStringExt, OsStrExt}, sync::OnceLock, time::{Duration, Instant}};
use windows_core::{PWSTR, BOOL};
use windows::{Win32::{Foundation::{CloseHandle, FILETIME, HWND, LPARAM, RECT, WPARAM}, Graphics::{Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS}, Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HMONITOR, MONITORINFOEXW}}, Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING}, System::Threading::{GetCurrentProcessId, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION}, UI::{Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK, WINEVENTPROC}, WindowsAndMessaging::{ClipCursor, DispatchMessageW, EnumWindows, GetAncestor, GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed, PostThreadMessageW, SetWindowPos, ShowWindow, TranslateMessage, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, GA_ROOT, GWL_EXSTYLE, HWND_NOTOPMOST, HWND_TOPMOST, MSG, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSENDCHANGING, SWP_NOSIZE, SWP_NOZORDER, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_TOOLWINDOW, WM_QUIT}}}};

const EVENT_REFRESH: u32 = 0xB001;
const REFRESH_INTERVAL: Duration = Duration::from_millis(700);
static EVENT_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static OWN_PROCESS: OnceLock<u32> = OnceLock::new();
static EVENT_QUEUE: OnceLock<Mutex<Vec<WindowEvent>>> = OnceLock::new();

#[derive(Clone, Copy)] struct WindowEvent { event: u32, hwnd: isize }

#[derive(Clone, Debug)]
pub struct WindowRow {
    pub id: isize,
    pub title: String,
    pub process: String,
    pub monitor: String,
    pub state: String,
    pub locked: bool,
    pub blacklisted: bool,
}
#[derive(Clone, Debug)]
pub struct MonitorRow { pub id: String, pub label: String }
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub windows: Vec<WindowRow>,
    pub monitors: Vec<MonitorRow>,
    pub status: String,
    pub mouse_target: Option<String>,
    /// Persisted blacklist entries, exposed as the canonical sorted list.
    pub blacklist: Vec<String>,
    pub revision: u64,
}
pub enum Command {
    Move(Vec<isize>, String), Lock(Vec<isize>), Unlock(Vec<isize>), Restore(Vec<isize>),
    ToggleTopmost(isize), AddBlacklist(String), RemoveBlacklist(String), MouseLock(Option<String>),
    UnlockAll, CancelPending(Vec<isize>), Refresh, SetAutostart(bool), Shutdown,
}
pub struct Backend { pub tx: mpsc::Sender<Command>, pub snapshot: Arc<Mutex<Snapshot>>, pub termination:Arc<Mutex<Option<thread::JoinHandle<()>>>> }

pub fn start(data_dir: PathBuf, readonly: bool) -> Backend {
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let (tx, rx) = mpsc::channel::<Command>();
    let output = snapshot.clone();
    let worker=thread::Builder::new().name("wlm-controller".into()).spawn(move || {
        let mut controller = Controller::new(data_dir, readonly, output, rx);
        controller.run();
    }).expect("start window controller");
    Backend { tx, snapshot, termination:Arc::new(Mutex::new(Some(worker))) }
}

pub(super) struct Controller {
    data_dir: PathBuf,
    readonly: bool,
    snapshot: Arc<Mutex<Snapshot>>,
    rx: mpsc::Receiver<Command>,
    monitors: Vec<MonitorInfo>,
    blacklist: HashSet<String>,
    deferred_locks: HashSet<isize>,
    lock_identity: HashMap<isize, (u32, u64)>,
    pub(super) locks: HashMap<isize, String>,
    tasks: scheduler::Scheduler,
    rules:Vec<rules::Rule>,
    event_thread: Option<EventThread>,
    mouse_target: Option<String>,
    last_status: String,
    revision: u64,
    dirty: bool,
}
impl Controller {
    pub(super) fn new(data_dir: PathBuf, readonly: bool, snapshot: Arc<Mutex<Snapshot>>, rx: mpsc::Receiver<Command>) -> Self {
        Self { data_dir, readonly, snapshot, rx, monitors: Vec::new(), blacklist: HashSet::new(), deferred_locks: HashSet::new(), lock_identity: HashMap::new(), locks: HashMap::new(), tasks: scheduler::Scheduler::default(), rules:Vec::new(), event_thread: None, mouse_target: None, last_status: "启动中".into(), revision: 0, dirty: true }
    }
    fn run(&mut self) {
        let _ = std::fs::create_dir_all(&self.data_dir);
        self.load_config();
        self.event_thread = Some(EventThread::start());
        self.refresh();
        let mut last_refresh = Instant::now();
        loop {
            match self.rx.recv_timeout(Duration::from_millis(25)) {
                Ok(Command::Shutdown) => break,
                Ok(command) => self.command(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {},
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let event_refresh = self.event_thread.as_ref().map(|e| e.take_refresh()).unwrap_or(false);
            self.consume_events();
            if self.dirty || event_refresh || last_refresh.elapsed() >= REFRESH_INTERVAL { self.refresh(); last_refresh = Instant::now(); }
            self.step_tasks();
        }
        if let Some(event) = self.event_thread.take() { event.stop(); }
        if !self.readonly { self.apply_mouse(None); }
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::Move(ids, target) => for id in ids { self.submit_move(id, target.clone(), false); },
            Command::Lock(ids) => for id in ids { self.submit_lock(id); },
            Command::Unlock(ids) => {for id in ids {self.remove_rule(id);self.locks.remove(&id);self.lock_identity.remove(&id);self.deferred_locks.remove(&id);self.tasks.cancel(scheduler::TaskId(id));} self.save_config();},
            Command::Restore(ids) => for id in ids { self.submit_restore(id); },
            Command::CancelPending(ids) => for id in ids { self.tasks.request_cancel(scheduler::TaskId(id)); },
            Command::ToggleTopmost(id) => self.toggle_topmost(id),
            Command::AddBlacklist(name) => { self.blacklist.insert(name.clone()); self.remove_blacklist_rules(&name); self.save_config(); self.dirty = true; },
            Command::RemoveBlacklist(name) => { self.blacklist.remove(&name); self.save_config(); self.dirty = true; },
            Command::MouseLock(target) => { self.mouse_target = target.clone(); self.apply_mouse(target); self.dirty = true; },
            Command::UnlockAll => { self.rules.clear();self.save_config(); self.locks.clear(); self.lock_identity.clear(); self.deferred_locks.clear(); self.tasks.clear(); self.mouse_target = None; self.apply_mouse(None); self.dirty = true; },
            Command::Refresh => self.refresh(),
            Command::SetAutostart(enabled) => { if !self.readonly { set_autostart(enabled); } },
            Command::Shutdown => {},
        }
    }
    fn submit_lock(&mut self, id: isize) {
        if self.blacklisted_id(id) { return; }
        let Some(hwnd) = self.read_window_state(id) else { return; };
        if hwnd.minimized {
            self.deferred_locks.insert(id);
            self.dirty = true;
            return;
        }
        let Some(target) = hwnd.actual_monitor else { return; };
        self.submit_move(id, target, true);
    }
    fn read_window_state(&self, id: isize) -> Option<WindowState> {
        read_window(HWND(id as *mut _), &self.monitors)
    }
    fn submit_move(&mut self, id: isize, target: String, locked: bool) {
        if self.blacklisted_id(id) || target.is_empty() { return; }
        if locked {
            self.locks.insert(id, target.clone());
            if let Some(state) = self.read_window_state(id) { self.lock_identity.insert(id, (state.process_id, state.process_created)); }
        }
        self.tasks.submit(scheduler::TaskId(id), scheduler::Action { kind: if locked { scheduler::ActionKind::Lock } else { scheduler::ActionKind::Move }, target });
    }
    fn submit_restore(&mut self, id: isize) {
        if self.blacklisted_id(id) { return; }
        self.tasks.submit(scheduler::TaskId(id), scheduler::Action::restore());
    }
    pub(super) fn finish_task(&mut self, task: &scheduler::Task) {
        if task.phase == scheduler::Phase::Done {
            if matches!(task.action.kind, scheduler::ActionKind::Move | scheduler::ActionKind::Lock | scheduler::ActionKind::LockCorrection) && self.locks.contains_key(&task.id.0) {
                self.locks.insert(task.id.0, task.action.target.clone());
                if let Some(state) = self.read_window_state(task.id.0) { self.lock_identity.insert(task.id.0, (state.process_id, state.process_created)); }
                self.remember_rule(task.id.0, &task.action.target);
                self.save_config();
            }
            self.last_status = format!("窗口 {} 已完成", task.id.0);
        } else if matches!(task.phase, scheduler::Phase::Failed | scheduler::Phase::Cancelled) {
            if matches!(task.action.kind, scheduler::ActionKind::Lock | scheduler::ActionKind::LockCorrection) {
                self.locks.remove(&task.id.0);
                self.lock_identity.remove(&task.id.0);
            }
            self.last_status = format!("窗口 {} {:?}", task.id.0, task.phase);
        }
        self.dirty = true;
        let message = format!("task hwnd={} phase={:?} target={} position_writes={} state_writes={}\n",task.id.0,task.phase,task.action.target,task.position_writes,task.state_writes);
        if !self.data_dir.as_os_str().is_empty() { use std::io::Write; if let Ok(mut f)=std::fs::OpenOptions::new().create(true).append(true).open(self.data_dir.join("backend.log")){let _=f.write_all(message.as_bytes());} }
    }
    fn step_tasks(&mut self) {
        let ids: Vec<_> = self.tasks.ids();
        for id in ids {
            let Some(mut task) = self.tasks.take(id) else { continue };
            if matches!(task.phase,scheduler::Phase::Done|scheduler::Phase::Failed|scheduler::Phase::Cancelled){self.finish_task(&task);continue;}
            if task.phase == scheduler::Phase::Backoff { if !task.ready(Instant::now()){self.tasks.put(task);continue;} task.phase=scheduler::Phase::Reading; }
            let ok = self.advance_task(&mut task);
            if !matches!(task.phase, scheduler::Phase::Done | scheduler::Phase::Failed | scheduler::Phase::Cancelled) {
                self.last_status = format!("窗口 {}：{:?}，第 {} 次尝试，位置写入 {} 次，状态写入 {} 次", task.id.0, task.phase, task.attempts.saturating_add(1), task.position_writes, task.state_writes);
            }
            if !ok {
                if task.phase == scheduler::Phase::Restoring {
                    task.phase = scheduler::Phase::Failed;
                } else if task.position_writes > 0 && (task.original_minimized || task.original_maximized) {
                    task.phase = scheduler::Phase::Restoring;
                    task.deadline = Instant::now() + Duration::from_millis(500);
                } else {
                    task.fail(Instant::now());
                }
            }
            if !matches!(task.phase, scheduler::Phase::Done | scheduler::Phase::Failed | scheduler::Phase::Cancelled) { self.tasks.put(task); } else { self.finish_task(&task); }
        }
    }
    fn advance_task(&mut self, task: &mut scheduler::Task) -> bool {
        if self.readonly { task.phase = scheduler::Phase::Done; return true; }
        let hwnd = HWND(task.id.0 as *mut _);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() { task.phase = scheduler::Phase::Failed; return true; }
        let before = match read_window(hwnd, &self.monitors) { Some(v) => v, None => return false };
        if task.phase == scheduler::Phase::Claimed { task.phase = scheduler::Phase::Reading; task.original_minimized = before.minimized; task.original_maximized = before.maximized; }
        if task.process_id == 0 { task.process_id = before.process_id; task.process_created = before.process_created; }
        if task.process_id != before.process_id || task.process_created != before.process_created { task.phase = scheduler::Phase::Failed; return true; }
        if task.phase == scheduler::Phase::Restoring { return self.restore_original_state(task, hwnd, before); }
        if task.action.kind == scheduler::ActionKind::Restore { return self.restore_window(task, hwnd, before); }
        let Some(target) = self.monitors.iter().find(|m| m.id == task.action.target).cloned() else { return false; };
        if task.cancel_requested && !task.position_written {
            if task.original_minimized && !before.minimized && !task.restored_final {
                unsafe { let _=ShowWindow(hwnd, SW_MINIMIZE); }
                task.restored_final = true;
                task.state_writes += 1;
                task.deadline = Instant::now() + Duration::from_millis(500);
                return true;
            }
            if task.original_maximized && !before.maximized && !task.restored_final {
                unsafe { ShowWindow(hwnd, SW_MAXIMIZE); }
                task.restored_final = true;
                task.state_writes += 1;
                task.deadline = Instant::now() + Duration::from_millis(500);
                return true;
            }
            if (!task.original_minimized || before.minimized) && (!task.original_maximized || before.maximized) {
                task.phase = scheduler::Phase::Cancelled;
                return true;
            }
            return Instant::now() < task.deadline;
        }
        if task.phase == scheduler::Phase::CancelRequested { task.phase = scheduler::Phase::WaitingStable; }
        if matches!(task.action.kind, scheduler::ActionKind::Lock | scheduler::ActionKind::LockCorrection) && before.minimized { task.phase = scheduler::Phase::Done; return true; }
        if task.phase == scheduler::Phase::Reading && !task.position_written && before.actual_monitor.as_deref() == Some(target.id.as_str()) && !before.minimized {
            task.observed_on_target = true;
            task.phase = scheduler::Phase::Done;
            return true;
        }
        if task.complete_if_observed(before.actual_monitor.as_deref(), before.minimized, before.maximized, &target.id) == scheduler::ObservationDecision::Complete { return true; }
        if (before.minimized || before.maximized) && !task.restored_once {
            let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
            task.restored_once = true;
            task.state_writes += 1;
            task.phase = scheduler::Phase::WaitingStable;
            task.deadline = Instant::now() + Duration::from_millis(500);
            return true;
        }
        if task.restored_once && !task.position_written && (before.minimized || before.maximized) {
            return Instant::now() < task.deadline;
        }
        if before.minimized && task.phase != scheduler::Phase::Verifying { return false; }
        if !task.position_written {
            let rect = target.work;
            let w = (before.rect.right - before.rect.left).max(1); let h = (before.rect.bottom - before.rect.top).max(1);
            let x = (rect.left + (rect.right - rect.left - w) / 2).clamp(rect.left, (rect.right - w).max(rect.left));
            let y = (rect.top + (rect.bottom - rect.top - h) / 2).clamp(rect.top, (rect.bottom - h).max(rect.top));
            if unsafe { SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOZORDER).is_err() } { return false; }
            task.position_written = true; task.position_writes += 1; task.phase = scheduler::Phase::WaitingStable; task.deadline = Instant::now() + Duration::from_millis(500); return true;
        }
        if task.phase == scheduler::Phase::WaitingStable {
            let now = read_window(hwnd, &self.monitors);
            if now.as_ref().and_then(|x| x.actual_monitor.as_deref()) == Some(target.id.as_str()) && now.as_ref().map(|x| !x.minimized).unwrap_or(false) {
                task.stable_samples = task.stable_samples.saturating_add(1);
                task.observed_on_target = true;
                if task.stable_samples >= 2 { task.phase = scheduler::Phase::Verifying; }
                return true;
            }
            task.stable_samples = 0;
            if Instant::now() < task.deadline { return true; }
            return false;
        }
        if task.phase == scheduler::Phase::Verifying {
            if task.original_minimized {
                if !task.restored_final { unsafe { let _=ShowWindow(hwnd, SW_MINIMIZE); } task.state_writes+=1; task.restored_final = true; task.deadline = Instant::now() + Duration::from_millis(500); return true; }
                if !before.minimized { return Instant::now() < task.deadline; }
            }
            if task.original_maximized {
                if !task.restored_final { unsafe { let _=ShowWindow(hwnd, SW_MAXIMIZE); } task.state_writes+=1; task.restored_final = true; task.deadline = Instant::now() + Duration::from_millis(500); return true; }
                if !before.maximized { return Instant::now() < task.deadline; }
            }
            if task.complete_if_observed(before.actual_monitor.as_deref(), before.minimized, before.maximized, &target.id) == scheduler::ObservationDecision::Complete { return true; }
            return false;
        }
        true
    }
    fn restore_original_state(&mut self, task: &mut scheduler::Task, hwnd: HWND, before: WindowState) -> bool {
        if task.original_minimized && !before.minimized && !task.restore_minimized_written {
            let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
            task.restore_minimized_written = true;
            task.state_writes += 1;
            task.deadline = Instant::now() + Duration::from_millis(500);
            return true;
        }
        if task.original_maximized && !before.maximized && !task.restore_maximized_written {
            let _ = unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
            task.restore_maximized_written = true;
            task.state_writes += 1;
            task.deadline = Instant::now() + Duration::from_millis(500);
            return true;
        }
        let minimized_ok = !task.original_minimized || before.minimized;
        let maximized_ok = !task.original_maximized || before.maximized;
        if minimized_ok && maximized_ok { task.phase = scheduler::Phase::Failed; return true; }
        Instant::now() < task.deadline
    }
    fn restore_window(&mut self, task: &mut scheduler::Task, hwnd: HWND, before: WindowState) -> bool {
        if task.phase == scheduler::Phase::Claimed { task.phase = scheduler::Phase::Reading; }
        // Restore is deliberately a minimize-only action. Maximized and normal
        // windows are already in the user's chosen state and must not be touched.
        if !before.minimized {
            task.phase = scheduler::Phase::Done;
            return true;
        }
        if !task.restored_once {
            let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
            task.restored_once = true;
            task.state_writes += 1;
            task.deadline = Instant::now() + Duration::from_millis(500);
            return true;
        }
        if unsafe { IsIconic(hwnd) }.as_bool() { return Instant::now() < task.deadline; }
        task.phase = scheduler::Phase::Done;
        true
    }
    fn refresh(&mut self) {
        self.monitors = enumerate_monitors();
        let rows = enumerate_windows(&self.monitors, &self.blacklist, &self.locks);
        if !self.readonly {
            let stale: Vec<_> = self.lock_identity.iter().filter_map(|(id, identity)| self.read_window_state(*id).filter(|state| (state.process_id, state.process_created) != *identity).map(|_| *id)).collect();
            for id in stale {
                self.locks.remove(&id);
                self.lock_identity.remove(&id);
                self.tasks.cancel(scheduler::TaskId(id));
            }
            let mut deferred = Vec::new();
            for id in self.deferred_locks.iter().copied() {
                if let Some(state) = self.read_window_state(id) {
                    if !state.minimized { deferred.push((id, state.actual_monitor)); }
                } else { deferred.push((id, None)); }
            }
            for (id, target) in deferred {
                self.deferred_locks.remove(&id);
                if let Some(target) = target { self.submit_move(id, target, true); }
            }
            let rule_matches: Vec<_> = rows.iter().filter(|row| !row.blacklisted && !self.locks.contains_key(&row.id)).filter_map(|row| self.rules.iter().find(|r| r.matches(&row.process, &row.title)).map(|r| (row.id, r.monitor.clone()))).collect();
            for (id, target) in rule_matches {
                if self.monitors.iter().any(|m| m.id == target) {
                    self.locks.insert(id, target.clone());
                    self.tasks.submit_lock_correction(scheduler::TaskId(id), &target);
                }
            }
            // Observe first; explicit tasks and active drags retain ownership.
            for (id,target) in self.locks.clone() {
                if let Some(state)=self.read_window_state(id) {
                    if !state.minimized && state.actual_monitor.as_deref()!=Some(target.as_str()) {
                        self.tasks.submit_lock_correction(scheduler::TaskId(id),&target);
                    }
                }
            }
        }
        self.revision += 1;
        if let Ok(mut snap) = self.snapshot.lock() {
            snap.monitors = self.monitors.iter().map(|m| MonitorRow { id: m.id.clone(), label: m.label.clone() }).collect();
            snap.windows = rows;
            snap.mouse_target = self.mouse_target.clone();
            snap.blacklist = {
                let mut values: Vec<_> = self.blacklist.iter().cloned().collect();
                values.sort();
                values
            };
            snap.status = if self.readonly { "只读预览".into() } else { self.last_status.clone() };
            snap.revision = self.revision;
        }
        self.dirty = false;
    }
    fn consume_events(&mut self) {
        let queue = EVENT_QUEUE.get_or_init(|| Mutex::new(Vec::new()));
        let events = queue.lock().map(|mut q| q.drain(..).collect::<Vec<_>>()).unwrap_or_default();
        for event in events {
            let id = scheduler::TaskId(event.hwnd);
            match event.event {
                EVENT_SYSTEM_MOVESIZESTART => self.tasks.note_drag(id, true),
                EVENT_SYSTEM_MOVESIZEEND => {
                    self.tasks.note_drag(id, false);
                    if let Some(target) = self.locks.get(&event.hwnd).cloned() { self.tasks.submit_lock_correction(id, &target); }
                }
                EVENT_OBJECT_DESTROY => { self.locks.remove(&event.hwnd); self.lock_identity.remove(&event.hwnd); self.deferred_locks.remove(&event.hwnd); self.tasks.cancel(id); }
                _ => {}
            }
        }
    }
    fn monitor_id(&self, id: isize) -> Option<String> { read_window(HWND(id as *mut _), &self.monitors).and_then(|w| w.actual_monitor) }
    fn blacklisted_id(&self, id: isize) -> bool { enumerate_windows(&self.monitors, &self.blacklist, &self.locks).into_iter().find(|w| w.id == id).map(|w| w.blacklisted).unwrap_or(false) }
    fn unlock_blacklisted(&mut self) { let ids: Vec<_> = self.locks.keys().copied().collect(); for id in ids { if self.blacklisted_id(id) { self.locks.remove(&id); self.tasks.cancel(scheduler::TaskId(id)); } } }
    fn remove_blacklist_rules(&mut self, entry: &str) {
        let needle = entry.to_lowercase();
        self.rules.retain(|rule| !rule.process.to_lowercase().contains(&needle) && !rule.title.to_lowercase().contains(&needle));
        let ids: Vec<_> = self.locks.keys().copied().collect();
        for id in ids {
            if let Some(row) = enumerate_windows(&self.monitors, &self.blacklist, &self.locks).into_iter().find(|row| row.id == id) {
                if row.process.to_lowercase().contains(&needle) || row.title.to_lowercase().contains(&needle) {
                    self.locks.remove(&id);
                    self.tasks.cancel(scheduler::TaskId(id));
                }
            }
        }
    }
    fn toggle_topmost(&mut self, id: isize) { if self.readonly { return; } let hwnd = HWND(id as *mut _); let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 }; let _ = unsafe { SetWindowPos(hwnd, if ex & 8 != 0 { Some(HWND_NOTOPMOST) } else { Some(HWND_TOPMOST) }, 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE) }; }
    fn apply_mouse(&self, target: Option<String>) { if self.readonly { return; } let rect = target.and_then(|id| self.monitors.iter().find(|m| m.id == id).map(|m| m.monitor)); if let Some(r) = rect { let _ = unsafe { ClipCursor(Some(&r)) }; } else { let _ = unsafe { ClipCursor(None) }; } }
    fn remember_rule(&mut self,id:isize,target:&str){let hwnd=HWND(id as *mut _);let title=title(hwnd);let mut pid=0;unsafe{GetWindowThreadProcessId(hwnd,Some(&mut pid));}if let Some(process)=process_name(pid){self.rules.retain(|r|!(r.process.eq_ignore_ascii_case(&process)&&r.title==title));self.rules.push(rules::Rule{process,title,monitor:target.into()});}}
    fn remove_rule(&mut self,id:isize){let hwnd=HWND(id as *mut _);let title=title(hwnd);let mut pid=0;unsafe{GetWindowThreadProcessId(hwnd,Some(&mut pid));}if let Some(process)=process_name(pid){self.rules.retain(|r|!r.matches(&process,&title));}}
    fn load_config(&mut self) {
        if let Ok(bytes)=std::fs::read(self.data_dir.join("config.json")) {
            if let Ok(value)=serde_json::from_slice::<serde_json::Value>(&bytes){
                if let Some(rules)=value.get("persistent_rules"){self.rules=serde_json::from_value(rules.clone()).unwrap_or_default();}
                if let Some(items)=value.get("blacklist").and_then(|x|x.as_array()){self.blacklist=items.iter().filter_map(|x|x.as_str().map(str::to_owned)).collect();}
            }
        }
    }
    fn save_config(&self) {
        if self.readonly{return;}
        let path=self.data_dir.join("config.json");let tmp=self.data_dir.join("config.pending.json");
        let mut value=std::fs::read(&path).ok().and_then(|b|serde_json::from_slice::<serde_json::Value>(&b).ok()).filter(|v|v.is_object()).unwrap_or_else(||serde_json::json!({}));
        value["persistent_rules"]=serde_json::json!(&self.rules);
        value["blacklist"]=serde_json::json!(self.blacklist.iter().collect::<Vec<_>>());
        if let Ok(bytes)=serde_json::to_vec_pretty(&value){if std::fs::write(&tmp,bytes).is_ok(){let source:Vec<u16>=tmp.as_os_str().encode_wide().chain(Some(0)).collect();let dest:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();let _=unsafe{MoveFileExW(windows_core::PCWSTR(source.as_ptr()),windows_core::PCWSTR(dest.as_ptr()),MOVEFILE_REPLACE_EXISTING)};}}
    }
}

#[derive(Clone)] struct MonitorInfo { id: String, label: String, monitor: RECT, work: RECT }
#[derive(Clone)] struct WindowState { rect: RECT, actual_monitor: Option<String>, minimized: bool, maximized: bool, process_id: u32, process_created: u64 }

fn enumerate_monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn callback(h: HMONITOR, _: windows::Win32::Graphics::Gdi::HDC, _: *mut RECT, data: LPARAM) -> BOOL { let list = &mut *(data.0 as *mut Vec<MonitorInfo>); let mut ex = windows::Win32::Graphics::Gdi::MONITORINFOEXW::default(); ex.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32; if GetMonitorInfoW(h, &mut ex.monitorInfo as *mut _).as_bool() { let name = utf16(&ex.szDevice); let id = if name.is_empty() { format!("MONITOR-{}", list.len()) } else { name.clone() }; list.push(MonitorInfo { id, label: name, monitor: ex.monitorInfo.rcMonitor, work: ex.monitorInfo.rcWork }); } BOOL(1) }
    let mut out = Vec::new(); unsafe { EnumDisplayMonitors(None, None, Some(callback), LPARAM(&mut out as *mut _ as isize)); } out
}
fn enumerate_windows(monitors: &[MonitorInfo], blacklist: &HashSet<String>, locks: &HashMap<isize, String>) -> Vec<WindowRow> {
    struct Ctx<'a> { out: Vec<WindowRow>, monitors: &'a [MonitorInfo], blacklist: &'a HashSet<String>, locks: &'a HashMap<isize, String> }
    unsafe extern "system" fn callback(hwnd: HWND, data: LPARAM) -> BOOL { let ctx = &mut *(data.0 as *mut Ctx<'_>); if let Some((row, _)) = window_row(hwnd, ctx.monitors, ctx.blacklist, ctx.locks) { ctx.out.push(row); } BOOL(1) }
    let mut ctx = Ctx { out: Vec::new(), monitors, blacklist, locks }; unsafe { let _ = EnumWindows(Some(callback), LPARAM(&mut ctx as *mut _ as isize)); } ctx.out.sort_by_key(|x| x.id); ctx.out
}
fn window_row(hwnd: HWND, monitors: &[MonitorInfo], blacklist: &HashSet<String>, locks: &HashMap<isize, String>) -> Option<(WindowRow, WindowState)> {
    let own = *OWN_PROCESS.get_or_init(|| unsafe { GetCurrentProcessId() }); let mut pid = 0; unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
    if pid == 0 || pid == own || (!unsafe { IsWindowVisible(hwnd) }.as_bool() && !unsafe { IsIconic(hwnd) }.as_bool()) { return None; }
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 }; if ex & WS_EX_TOOLWINDOW.0 != 0 { return None; }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) }; if root.0 != hwnd.0 { return None; }
    let mut cloaked = 0u32; if unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4) }.is_ok() && cloaked != 0 { return None; }
    let title = title(hwnd); if title.trim().is_empty() { return None; }
    let process = process_name(pid).unwrap_or_else(|| format!("pid:{pid}")); let bl = blacklist.iter().any(|x| { let x=x.to_lowercase(); !x.is_empty()&&(process.to_lowercase().contains(&x)||title.to_lowercase().contains(&x)) });
    let state = if unsafe { IsIconic(hwnd) }.as_bool() { "最小化" } else if unsafe { IsZoomed(hwnd) }.as_bool() { "最大化" } else { "正常" }.to_string();
    let actual = read_window(hwnd, monitors)?; let lock = locks.get(&(hwnd.0 as isize));
    let state = match lock { Some(target) => format!("{state} · 锁定目标 {target}"), None => state };
    Some((WindowRow { id: hwnd.0 as isize, title, process, monitor: actual.actual_monitor.clone().unwrap_or_else(|| "未知".into()), state, locked: lock.is_some(), blacklisted: bl }, actual))
}
fn read_window(hwnd: HWND, monitors: &[MonitorInfo]) -> Option<WindowState> {
    let mut rect = RECT::default(); unsafe { GetWindowRect(hwnd, &mut rect).ok()?; }
    let mut extended = rect;
    let _ = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut extended as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32) };
    let minimized = unsafe { IsIconic(hwnd) }.as_bool();
    let actual_monitor = if minimized { None } else { monitor_for_rect(extended, monitors).map(|m| m.id.clone()) };
    let (process_id, process_created) = process_identity(hwnd)?;
    Some(WindowState { rect: extended, actual_monitor, minimized, maximized: unsafe { IsZoomed(hwnd) }.as_bool(), process_id, process_created })
}
fn monitor_for_rect(rect: RECT, monitors: &[MonitorInfo]) -> Option<&MonitorInfo> {
    monitors.iter().max_by_key(|m| {
        let left = rect.left.max(m.monitor.left); let top = rect.top.max(m.monitor.top);
        let right = rect.right.min(m.monitor.right); let bottom = rect.bottom.min(m.monitor.bottom);
        i64::from((right - left).max(0)) * i64::from((bottom - top).max(0))
    })
}
fn process_identity(hwnd: HWND) -> Option<(u32, u64)> {
    unsafe {
        let mut pid = 0; GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 { return None; }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut created = FILETIME::default(); let mut exited = FILETIME::default(); let mut kernel = FILETIME::default(); let mut user = FILETIME::default();
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        if ok { Some((pid, (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))) } else { None }
    }
}
fn title(hwnd: HWND) -> String { let n = unsafe { GetWindowTextLengthW(hwnd) }; if n <= 0 { return String::new(); } let mut b = vec![0u16; n as usize + 1]; let len = unsafe { GetWindowTextW(hwnd, &mut b) }; String::from_utf16_lossy(&b[..len.max(0) as usize]) }
fn process_name(pid: u32) -> Option<String> { unsafe { let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?; let mut b = vec![0u16; 1024]; let mut n = b.len() as u32; let ok = QueryFullProcessImageNameW(h, windows::Win32::System::Threading::PROCESS_NAME_WIN32, PWSTR(b.as_mut_ptr()), &mut n).is_ok(); let _ = CloseHandle(h); if ok { Some(std::path::Path::new(&OsString::from_wide(&b[..n as usize])).file_name()?.to_string_lossy().into_owned()) } else { None } } }
fn utf16(v: &[u16]) -> String { let n = v.iter().position(|x| *x == 0).unwrap_or(v.len()); String::from_utf16_lossy(&v[..n]) }

fn set_autostart(enabled: bool) {
    let Ok(exe) = std::env::current_exe() else { return; };
    let action = if enabled { "ADD" } else { "DELETE" };
    let mut cmd = std::process::Command::new("reg.exe");
    cmd.args([action, r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "WindowLockMasterRust"]);
    if enabled { cmd.args(["/t", "REG_SZ", "/d", &format!("\"{}\"", exe.display()), "/f"]); }
    let _ = cmd.status();
}

pub mod scheduler {
    use std::{collections::HashMap, time::{Duration, Instant}};

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] pub struct TaskId(pub isize);
    #[derive(Clone, Copy, Debug, Eq, PartialEq)] pub enum ActionKind { Move, Lock, Restore, LockCorrection }
    #[derive(Clone, Debug, Eq, PartialEq)] pub struct Action { pub kind: ActionKind, pub target: String }
    impl Action {
        pub fn move_to(target: &str) -> Self { Self { kind: ActionKind::Move, target: target.into() } }
        pub fn lock_to(target: &str) -> Self { Self { kind: ActionKind::Lock, target: target.into() } }
        fn correction(target: &str) -> Self { Self { kind: ActionKind::LockCorrection, target: target.into() } }
        pub fn restore() -> Self { Self { kind: ActionKind::Restore, target: String::new() } }
        fn is_correction(&self) -> bool { self.kind == ActionKind::LockCorrection }
    }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)] pub enum Phase { Claimed, Reading, WaitingStable, Verifying, Restoring, Backoff, CancelRequested, Done, Cancelled, Failed }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)] pub enum ObservationDecision { NotReady, Complete }
    #[derive(Clone, Debug)] pub struct Task {
        pub id: TaskId, pub action: Action, pub generation: u64, pub phase: Phase,
        pub attempts: u8, pub next_attempt: Instant, pub deadline: Instant,
        pub original_minimized: bool, pub original_maximized: bool,
        pub restored_once: bool, pub restored_final: bool, pub restore_minimized_written: bool, pub restore_maximized_written: bool, pub position_written: bool,
        pub observed_on_target: bool, pub stable_samples: u8, pub position_writes: u32, pub state_writes: u32,
        pub process_id: u32, pub process_created: u64, pub cancel_requested: bool,
    }
    impl Task {
        /// Completes only after the controller has read the required real state.
        /// There is intentionally no unconditional `advance` operation.
        pub fn complete_if_observed(&mut self, monitor: Option<&str>, minimized: bool, maximized: bool, target: &str) -> ObservationDecision {
            let on_target = if self.original_minimized && minimized { self.observed_on_target && self.position_written && self.restored_final } else { monitor == Some(target) && !minimized };
            match self.phase {
                Phase::Reading if on_target && !self.original_minimized => {
                    self.observed_on_target = true;
                    self.phase = if self.cancel_requested { Phase::Cancelled } else { Phase::Done };
                    ObservationDecision::Complete
                }
                Phase::Verifying => {
                    let restored = (!self.original_minimized || minimized) && (!self.original_maximized || maximized);
                    if on_target && restored { self.phase = if self.cancel_requested { Phase::Cancelled } else { Phase::Done }; ObservationDecision::Complete } else { ObservationDecision::NotReady }
                }
                _ => ObservationDecision::NotReady,
            }
        }
        pub fn fail(&mut self, now: Instant) {
            self.attempts = self.attempts.saturating_add(1);
            if self.attempts >= 3 { self.phase = Phase::Failed; }
            else { self.position_written=false; self.observed_on_target=false; self.stable_samples=0; self.restored_final=false; self.restored_once=false; self.restore_minimized_written=false; self.restore_maximized_written=false; self.phase = Phase::Backoff; self.next_attempt = now + Duration::from_millis(250 * (1u64 << (self.attempts - 1))); }
        }
        pub fn ready(&self, now: Instant) -> bool { now >= self.next_attempt }
    }
    #[derive(Default)] pub struct Scheduler { tasks: HashMap<TaskId, Task>, generations: HashMap<TaskId, u64>, dragging: HashMap<TaskId, bool> }
    impl Scheduler {
        /// Same destination/kind is a coalesced request: keep the task's original
        /// state, generation, and write counters instead of restarting it.
        pub fn submit(&mut self, id: TaskId, action: Action) -> u64 {
            if let Some(existing) = self.tasks.get_mut(&id) {
                if existing.action == action { return existing.generation; }
                if action.is_correction() && !existing.action.is_correction() { return existing.generation; }
                // An explicit request may supersede a lock request, but a
                // same-target merge keeps the ownership, original state,
                // generation and already-issued write counters.
                if existing.action.target == action.target && !action.is_correction() {
                    existing.action = action;
                    return existing.generation;
                }
            }
            let generation = self.generations.entry(id).and_modify(|x| *x += 1).or_insert(1);
            let generation = *generation;
            self.tasks.insert(id, Task { id, action, generation, phase: Phase::Claimed, attempts: 0, next_attempt: Instant::now(), deadline: Instant::now(), original_minimized: false, original_maximized: false, restored_once: false, restored_final: false, restore_minimized_written: false, restore_maximized_written: false, position_written: false, observed_on_target: false, stable_samples: 0, position_writes: 0, state_writes: 0, process_id: 0, process_created: 0, cancel_requested: false });
            generation
        }
        pub fn submit_lock_correction(&mut self, id: TaskId, target: &str) -> u64 {
            if self.dragging.get(&id).copied().unwrap_or(false) { return self.generation(id); }
            self.submit(id, Action::correction(target))
        }
        pub fn note_drag(&mut self, id: TaskId, dragging: bool) { self.dragging.insert(id, dragging); if dragging && self.tasks.get(&id).map(|t| t.action.is_correction()).unwrap_or(false) { self.tasks.remove(&id); } }
        pub fn request_cancel(&mut self, id: TaskId) {
            if let Some(task) = self.tasks.get_mut(&id) {
                if task.position_writes == 0 && !task.restored_once && !matches!(task.phase, Phase::Done | Phase::Failed) { task.phase = Phase::Cancelled; }
                else { task.cancel_requested = true; task.phase = Phase::CancelRequested; }
            }
        }
        pub fn take(&mut self, id: TaskId) -> Option<Task> { self.tasks.remove(&id) }
        pub fn put(&mut self, task: Task) { if task.phase != Phase::Cancelled { self.tasks.insert(task.id, task); } }
        pub fn cancel(&mut self, id: TaskId) { self.request_cancel(id); }
        pub fn clear(&mut self) { let ids = self.ids(); for id in ids { self.request_cancel(id); } }
        pub fn ids(&self) -> Vec<TaskId> { self.tasks.keys().copied().collect() }
        pub fn pending_len(&self) -> usize { self.tasks.len() }
        pub fn generation(&self, id: TaskId) -> u64 { self.generations.get(&id).copied().unwrap_or(0) }
        pub fn is_current(&self, id: TaskId, generation: u64) -> bool { self.generation(id) == generation }
    }
}

struct EventThread { thread_id: u32, stop: Arc<std::sync::atomic::AtomicBool>, refresh: Arc<std::sync::atomic::AtomicBool>, join: Option<thread::JoinHandle<()>> }
impl EventThread {
    fn start() -> Self { let stop = Arc::new(std::sync::atomic::AtomicBool::new(false)); let refresh = Arc::new(std::sync::atomic::AtomicBool::new(true)); let stop2 = stop.clone(); let refresh2 = refresh.clone(); let (ready_tx, ready_rx) = mpsc::channel(); let join = thread::Builder::new().name("wlm-winevent".into()).spawn(move || unsafe { let tid = windows::Win32::System::Threading::GetCurrentThreadId(); let _ = ready_tx.send(tid); let callback: WINEVENTPROC = Some(event_callback); let hook = SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART.min(EVENT_OBJECT_CREATE), EVENT_OBJECT_LOCATIONCHANGE.max(EVENT_SYSTEM_MOVESIZEEND), None, callback, 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS); let mut msg = MSG::default(); while !stop2.load(std::sync::atomic::Ordering::Relaxed) { if GetMessageW(&mut msg, None, 0, 0).0 <= 0 { break; } if msg.message == EVENT_REFRESH { refresh2.store(true, std::sync::atomic::Ordering::Relaxed); } else { TranslateMessage(&msg); DispatchMessageW(&msg); } } if !hook.0.is_null() { let _ = UnhookWinEvent(hook); } }); let thread_id = ready_rx.recv_timeout(Duration::from_secs(1)).unwrap_or(0); Self { thread_id, stop, refresh, join: Some(join.expect("start event thread")) } }
    fn take_refresh(&self) -> bool { self.refresh.swap(false, std::sync::atomic::Ordering::Relaxed) || EVENT_DIRTY.swap(false, std::sync::atomic::Ordering::Relaxed) }
    fn stop(mut self) { self.stop.store(true, std::sync::atomic::Ordering::Relaxed); if self.thread_id != 0 { unsafe { let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)); } } if let Some(j) = self.join.take() { let _ = j.join(); } }
}
unsafe extern "system" fn event_callback(_: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    if object != 0 || child != 0 || hwnd.0.is_null() || !matches!(event, EVENT_SYSTEM_MOVESIZESTART | EVENT_SYSTEM_MOVESIZEEND | EVENT_OBJECT_CREATE | EVENT_OBJECT_DESTROY | EVENT_OBJECT_SHOW | EVENT_OBJECT_HIDE | EVENT_OBJECT_LOCATIONCHANGE) { return; }
    let queue = EVENT_QUEUE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut q) = queue.lock() {
        let id = hwnd.0 as isize;
        // LOCATIONCHANGE can arrive for every pixel.  Coalesce it per HWND;
        // the controller's bounded refresh still reads the real final state.
        let duplicate = q.iter().any(|item| item.hwnd == id && item.event == event);
        if !duplicate && q.len() < 1024 { q.push(WindowEvent { event, hwnd: id }); }
    }
    EVENT_DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
}
