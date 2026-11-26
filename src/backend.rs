//! Real Win32 backend for WindowLockMasterRust.
//!
//! The UI only sees `Snapshot`.  All HWND reads and writes are serialized on
//! the controller thread; WinEvent callbacks never touch a window.

use std::{path::PathBuf, sync::{mpsc, Arc, Mutex}, thread};

#[path = "backend/platform.rs"]
mod platform;

pub use platform::{Command, Backend, MonitorRow, Snapshot, WindowRow};

/// Starts the read/write controller and its read-only event watcher.
///
/// `readonly` still enumerates real windows and monitors, but every operation
/// that could change a window, the mouse clip, the registry, or the config is
/// suppressed.  This is deliberately useful for UI development.
pub fn start(data_dir: PathBuf, readonly: bool) -> Backend {
    platform::start(data_dir, readonly)
}

#[cfg(test)]
mod tests {
    use super::platform::scheduler::{Action, ActionKind, ObservationDecision, Phase, Scheduler, TaskId};
    use std::time::{Duration, Instant};

    #[test]
    fn duplicate_same_request_preserves_generation_state_and_write_count() {
        let mut s = Scheduler::default();
        let id = TaskId(7);
        let generation = s.submit(id, Action::move_to("DISPLAY2"));
        let mut task = s.take(id).unwrap();
        task.phase = Phase::WaitingStable;
        task.position_written = true;
        task.position_writes = 1;
        s.put(task);

        let same_generation = s.submit(id, Action::move_to("DISPLAY2"));
        let task = s.take(id).unwrap();
        assert_eq!(same_generation, generation);
        assert_eq!(task.generation, generation);
        assert_eq!(task.phase, Phase::WaitingStable);
        assert_eq!(task.position_writes, 1);
        assert_eq!(task.action.kind, ActionKind::Move);
    }

    #[test]
    fn lock_correction_cannot_preempt_explicit_task_but_new_explicit_move_can() {
        let mut s = Scheduler::default();
        let id = TaskId(1);
        let generation = s.submit(id, Action::move_to("DISPLAY3"));
        assert_eq!(s.submit_lock_correction(id, "DISPLAY1"), generation);
        let task = s.take(id).unwrap();
        assert_eq!(task.action.kind, ActionKind::Move);
        assert_eq!(task.action.target, "DISPLAY3");

        let new_generation = s.submit(id, Action::move_to("DISPLAY4"));
        let task = s.take(id).unwrap();
        assert!(new_generation > generation);
        assert_eq!(task.action.target, "DISPLAY4");
    }

    #[test]
    fn real_observation_drives_completion_and_wrong_state_stays_pending() {
        let mut s = Scheduler::default();
        let id = TaskId(2);
        s.submit(id, Action::move_to("DISPLAY1"));
        let mut task = s.take(id).unwrap();
        task.phase = Phase::Reading;
        assert_eq!(task.complete_if_observed(Some("DISPLAY2"), false, false, "DISPLAY1"), ObservationDecision::NotReady);
        assert_eq!(task.phase, Phase::Reading);
        assert_eq!(task.position_writes, 0);
        assert_eq!(task.complete_if_observed(Some("DISPLAY1"), false, false, "DISPLAY1"), ObservationDecision::Complete);
        assert_eq!(task.phase, Phase::Done);
        assert_eq!(task.position_writes, 0);
    }

    #[test]
    fn cancel_pending_is_immediate_before_write_and_safe_after_write() {
        let mut s = Scheduler::default();
        let id = TaskId(3);
        s.submit(id, Action::move_to("DISPLAY2"));
        s.request_cancel(id);
        assert_eq!(s.take(id).unwrap().phase, Phase::Cancelled);

        s.submit(id, Action::move_to("DISPLAY2"));
        let mut task = s.take(id).unwrap();
        task.phase = Phase::WaitingStable;
        task.position_written = true;
        task.position_writes = 1;
        s.put(task);
        s.request_cancel(id);
        let mut task = s.take(id).unwrap();
        assert_eq!(task.phase, Phase::CancelRequested);
        task.phase = Phase::Verifying;
        task.restored_final = true;
        assert_eq!(task.complete_if_observed(Some("DISPLAY2"), false, false, "DISPLAY2"), ObservationDecision::Complete);
        assert_eq!(task.phase, Phase::Cancelled);
    }

    #[test]
    fn drag_end_allows_one_correction_but_drag_events_do_not() {
        let mut s = Scheduler::default();
        let id = TaskId(4);
        s.note_drag(id, true);
        s.submit_lock_correction(id, "DISPLAY2");
        assert_eq!(s.pending_len(), 0);
        s.note_drag(id, false);
        s.submit_lock_correction(id, "DISPLAY2");
        assert_eq!(s.pending_len(), 1);
        assert_eq!(s.take(id).unwrap().action.kind, ActionKind::LockCorrection);
    }

    #[test]
    fn completed_locked_move_commits_new_binding() {
        use super::platform::{Controller, Snapshot};
        use std::{collections::HashMap, sync::{mpsc,Arc,Mutex}};
        let (_,rx)=mpsc::channel();
        let mut c=Controller::new(std::path::PathBuf::new(),true,Arc::new(Mutex::new(Snapshot::default())),rx);
        c.locks=HashMap::from([(99,"DISPLAY1".into())]);
        let mut s=Scheduler::default();s.submit(TaskId(99),Action::move_to("DISPLAY2"));
        let mut task=s.take(TaskId(99)).unwrap();task.phase=Phase::Done;
        c.finish_task(&task);
        assert_eq!(c.locks.get(&99).map(String::as_str),Some("DISPLAY2"));
    }

    #[test]
    fn retry_after_position_timeout_can_issue_another_bounded_move() {
        let mut s=Scheduler::default();s.submit(TaskId(11),Action::move_to("DISPLAY2"));
        let mut t=s.take(TaskId(11)).unwrap();t.position_written=true;t.position_writes=1;t.phase=Phase::WaitingStable;
        t.fail(Instant::now());
        assert!(!t.position_written,"retry must permit a new position command");
        assert_eq!(t.position_writes,1,"must retain audit counters");
    }

    #[test]
    fn retry_uses_bounded_backoff_and_never_spins() {
        let mut s = Scheduler::default();
        s.submit(TaskId(5), Action::move_to("DISPLAY2"));
        let mut task = s.take(TaskId(5)).unwrap();
        let now = Instant::now();
        task.fail(now);
        assert_eq!(task.phase, Phase::Backoff);
        assert!(task.next_attempt >= now + Duration::from_millis(250));
        assert!(!task.ready(now + Duration::from_millis(249)));
        assert!(task.ready(task.next_attempt));
        task.fail(task.next_attempt);
        assert!(task.attempts <= 3);
    }
}
