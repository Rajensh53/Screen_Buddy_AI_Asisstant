//! Privacy gate: decides whether a selection may be read and explained.
//! Pure logic, no Windows calls, so every rule is unit-tested.
//!
//! Called twice per gesture:
//! - `check_apps` before any UI Automation call, with the apps under the gesture points,
//!   so blocked apps and our own windows are never read.
//! - `check_element` after reading, with the process that owns the text (which can be a
//!   helper process such as msedgewebview2.exe) and whether it is a password field.

use std::collections::HashSet;

/// A process as the gate sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppIdentity<'a> {
    pub process_id: u32,
    /// Exe file name, for example `chrome.exe`. Empty if unknown.
    pub process_name: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Skip(SkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The mascot, popup or settings window. Skipped silently.
    OwnWindow,
    /// A password field. Its text is never read.
    PasswordField,
    /// An app on the block list (lowercase exe name).
    BlockedApp(String),
}

/// Apps whose text is never read. Stored as lowercase exe names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockList {
    apps: HashSet<String>,
}

/// Password managers and the Windows credential prompt.
const DEFAULT_BLOCKED: &[&str] = &[
    "1password.exe",
    "bitwarden.exe",
    "keepass.exe",
    "keepassxc.exe",
    "enpass.exe",
    "nordpass.exe",
    "credentialuibroker.exe",
];

impl BlockList {
    /// Builds a list from user entries: trims spaces, ignores case, adds `.exe` when missing
    /// and skips blank entries.
    pub fn new(entries: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let apps = entries.into_iter().filter_map(|e| normalize(e.as_ref())).collect();
        Self { apps }
    }

    pub fn defaults() -> Self {
        Self::new(DEFAULT_BLOCKED)
    }

    /// Adds more entries, for example a debug override.
    #[cfg_attr(not(debug_assertions), allow(dead_code, reason = "only the debug override uses it"))]
    pub fn extend(&mut self, entries: impl IntoIterator<Item = impl AsRef<str>>) {
        self.apps.extend(entries.into_iter().filter_map(|e| normalize(e.as_ref())));
    }

    /// The normalized entries, sorted, for showing and saving.
    pub fn entries(&self) -> Vec<String> {
        let mut entries: Vec<String> = self.apps.iter().cloned().collect();
        entries.sort();
        entries
    }

    pub fn contains(&self, process_name: &str) -> bool {
        normalize(process_name).is_some_and(|name| self.apps.contains(&name))
    }
}

fn normalize(entry: &str) -> Option<String> {
    let name = entry.trim().to_lowercase();
    if name.is_empty() {
        return None;
    }
    Some(if name.ends_with(".exe") { name } else { format!("{name}.exe") })
}

#[derive(Debug, Clone)]
pub struct PrivacyGate {
    own_pid: u32,
    block_list: BlockList,
}

impl PrivacyGate {
    pub fn new(own_pid: u32, block_list: BlockList) -> Self {
        Self { own_pid, block_list }
    }

    /// Before reading: the apps under the gesture points (for a drag, where it started and ended).
    pub fn check_apps(&self, apps: &[AppIdentity]) -> Decision {
        if apps.iter().any(|a| a.process_id == self.own_pid) {
            return Decision::Skip(SkipReason::OwnWindow);
        }
        self.blocked(apps)
    }

    /// After reading: the process that owns the text, and whether it is a password field.
    pub fn check_element(&self, element: AppIdentity, is_password: bool) -> Decision {
        if element.process_id == self.own_pid {
            return Decision::Skip(SkipReason::OwnWindow);
        }
        if is_password {
            return Decision::Skip(SkipReason::PasswordField);
        }
        self.blocked(&[element])
    }

    fn blocked(&self, apps: &[AppIdentity]) -> Decision {
        match apps.iter().find(|a| self.block_list.contains(a.process_name)) {
            Some(app) => Decision::Skip(SkipReason::BlockedApp(app.process_name.to_lowercase())),
            None => Decision::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: u32 = 100;

    fn app(process_id: u32, process_name: &str) -> AppIdentity<'_> {
        AppIdentity { process_id, process_name }
    }

    fn gate() -> PrivacyGate {
        PrivacyGate::new(OWN, BlockList::defaults())
    }

    fn blocked(name: &str) -> Decision {
        Decision::Skip(SkipReason::BlockedApp(name.into()))
    }

    #[test]
    fn normal_app_is_allowed() {
        assert_eq!(gate().check_apps(&[app(1, "chrome.exe"), app(1, "chrome.exe")]), Decision::Allow);
        assert_eq!(gate().check_element(app(2, "msedgewebview2.exe"), false), Decision::Allow);
    }

    #[test]
    fn own_window_at_any_gesture_point_is_skipped() {
        // Drag that starts on the mascot and ends over another app.
        assert_eq!(
            gate().check_apps(&[app(OWN, "screen-buddy.exe"), app(1, "notepad.exe")]),
            Decision::Skip(SkipReason::OwnWindow)
        );
    }

    #[test]
    fn own_process_owning_the_text_is_skipped() {
        assert_eq!(gate().check_element(app(OWN, "screen-buddy.exe"), false), Decision::Skip(SkipReason::OwnWindow));
    }

    #[test]
    fn password_field_is_skipped_in_an_allowed_app() {
        assert_eq!(gate().check_element(app(1, "chrome.exe"), true), Decision::Skip(SkipReason::PasswordField));
    }

    #[test]
    fn blocked_app_is_skipped_ignoring_case() {
        assert_eq!(gate().check_apps(&[app(1, "1Password.exe")]), blocked("1password.exe"));
        assert_eq!(gate().check_apps(&[app(1, "BITWARDEN.EXE")]), blocked("bitwarden.exe"));
    }

    #[test]
    fn entry_without_exe_matches() {
        let gate = PrivacyGate::new(OWN, BlockList::new(["keepass"]));
        assert_eq!(gate.check_apps(&[app(1, "KeePass.exe")]), blocked("keepass.exe"));
    }

    #[test]
    fn blocked_helper_process_is_skipped_after_reading() {
        let gate = PrivacyGate::new(OWN, BlockList::new(["bankhelper.exe"]));
        assert_eq!(gate.check_element(app(5, "BankHelper.exe"), false), blocked("bankhelper.exe"));
    }

    #[test]
    fn own_window_beats_password_and_block_list() {
        let gate = PrivacyGate::new(OWN, BlockList::new(["screen-buddy.exe"]));
        assert_eq!(gate.check_element(app(OWN, "screen-buddy.exe"), true), Decision::Skip(SkipReason::OwnWindow));
    }

    #[test]
    fn password_beats_block_list() {
        assert_eq!(gate().check_element(app(1, "1password.exe"), true), Decision::Skip(SkipReason::PasswordField));
    }

    #[test]
    fn blank_entries_are_ignored_and_spaces_trimmed() {
        let list = BlockList::new(["", "   ", "  Notepad.exe  "]);
        assert_eq!(list, BlockList::new(["notepad.exe"]));
        assert!(list.contains("notepad.exe"));
    }

    #[test]
    fn empty_block_list_allows_everything() {
        let gate = PrivacyGate::new(OWN, BlockList::default());
        assert_eq!(gate.check_apps(&[app(1, "1password.exe")]), Decision::Allow);
    }

    #[test]
    fn unknown_process_name_is_not_blocked() {
        assert!(!BlockList::defaults().contains(""));
        assert_eq!(gate().check_apps(&[app(1, "")]), Decision::Allow);
    }

    #[test]
    fn defaults_contain_password_managers() {
        let list = BlockList::defaults();
        for name in ["1password.exe", "bitwarden.exe", "keepass.exe", "keepassxc.exe", "credentialuibroker.exe"] {
            assert!(list.contains(name), "{name} should be blocked by default");
        }
        assert!(!list.contains("notepad.exe"));
    }

    #[test]
    fn entries_are_normalized_sorted_and_unique() {
        let list = BlockList::new(["Notepad", "notepad.exe", " B.exe "]);
        assert_eq!(list.entries(), ["b.exe", "notepad.exe"]);
    }

    #[test]
    fn extend_adds_entries() {
        let mut list = BlockList::defaults();
        list.extend(["notepad"]);
        assert!(list.contains("Notepad.exe"));
        assert!(list.contains("bitwarden.exe"));
    }
}
