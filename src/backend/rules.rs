use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Rule {
    pub process: String,
    pub title: String,
    pub monitor: String,
}

impl Rule {
    pub fn matches(&self, process: &str, title: &str) -> bool {
        self.process.eq_ignore_ascii_case(process) && !self.title.is_empty() && title.contains(&self.title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_requires_process_and_title() {
        let r = Rule { process: "demo.exe".into(), title: "Editor".into(), monitor: "m".into() };
        assert!(r.matches("DEMO.EXE", "Editor 1"));
        assert!(!r.matches("other.exe", "Editor 1"));
        assert!(!r.matches("demo.exe", "Another"));
    }

    #[test]
    fn rule_title_matching_is_case_sensitive_but_process_is_not() {
        let r = Rule { process: "demo.exe".into(), title: "Editor".into(), monitor: "m".into() };
        assert!(r.matches("DEMO.EXE", "Editor - file"));
        assert!(!r.matches("DEMO.EXE", "editor - file"));
    }
}