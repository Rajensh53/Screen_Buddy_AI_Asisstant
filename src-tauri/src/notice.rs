//! Short messages shown in the popup when Screen Buddy can't explain something, so the user
//! never sees "nothing happened". All user-facing texts live here, in one place. Pure logic.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Every notice is shown, except the very same one again within this short time (one gesture
/// can be reported twice, for example by a double-click right after a drag). Gestures on a
/// window's frame (moving or resizing it) never reach the notices at all.
pub const REPEAT_AFTER: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    CantRead,
    NothingSelected,
    PasswordField,
    BlockedApp,
    ClipboardSync,
    OllamaNotRunning,
    ModelMissing,
    HookFailed,
    UiaUnavailable,
}

/// A button on the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeAction {
    OpenSettings,
    Retry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub kind: NoticeKind,
    pub title: String,
    pub detail: String,
    pub action: Option<NoticeAction>,
}

/// What a notice is about. Only the fields its kind needs are used.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context<'a> {
    /// The app's display name or exe name.
    pub app: Option<&'a str>,
    pub model: Option<&'a str>,
    /// Why it failed: for startup problems the technical reason, for selections why copying
    /// didn't work (`selection::clipboard` errors, phrased to follow "copying didn't work: ").
    pub reason: Option<&'a str>,
    /// At startup there is no selection to retry, so Ollama notices offer Settings instead.
    pub at_startup: bool,
}

pub fn notice(kind: NoticeKind, cx: Context) -> Notice {
    let app = cx.app.filter(|a| !a.trim().is_empty()).unwrap_or("This app");
    let model = cx.model.unwrap_or("the model");
    let ollama_action = if cx.at_startup { NoticeAction::OpenSettings } else { NoticeAction::Retry };
    let (title, detail, action) = match kind {
        NoticeKind::CantRead => (
            "Can't read this app".to_owned(),
            match cx.reason {
                Some(reason) => format!("{app} doesn't share its text, and copying didn't work: {reason}."),
                None => format!("{app} doesn't share its text."),
            },
            None,
        ),
        NoticeKind::NothingSelected => (
            "No selected text found".to_owned(),
            match cx.reason {
                Some(reason) => format!("{app} didn't share the selection, and copying didn't work: {reason}. Select the text again."),
                None => format!("{app} didn't share the selection. Select the text again."),
            },
            None,
        ),
        NoticeKind::PasswordField => (
            "Password fields are never read".to_owned(),
            "Screen Buddy skips them to keep your passwords private.".to_owned(),
            None,
        ),
        NoticeKind::BlockedApp => (
            format!("Turned off for {app}"),
            "This app is on your block list. You can change this in Settings.".to_owned(),
            Some(NoticeAction::OpenSettings),
        ),
        NoticeKind::ClipboardSync => (
            "Can't read this app safely".to_owned(),
            format!(
                "{app} can only be read through the clipboard, and your clipboard syncs across devices. \
                 Turn off clipboard sync in Windows Settings → System → Clipboard."
            ),
            Some(NoticeAction::OpenSettings),
        ),
        NoticeKind::OllamaNotRunning => (
            "Ollama is not running".to_owned(),
            "Start Ollama, then try again.".to_owned(),
            Some(ollama_action),
        ),
        NoticeKind::ModelMissing => (
            format!("Model {model} is not installed"),
            format!("Run: ollama pull {model}"),
            Some(ollama_action),
        ),
        NoticeKind::HookFailed | NoticeKind::UiaUnavailable => (
            "Screen Buddy can't watch selections".to_owned(),
            match cx.reason {
                Some(reason) => format!("{reason}. Restart Screen Buddy."),
                None => "Restart Screen Buddy.".to_owned(),
            },
            None,
        ),
    };
    Notice { kind, title, detail, action }
}

/// Remembers when each notice was last shown, per app.
#[derive(Debug, Default)]
pub struct NoticeThrottle {
    shown: HashMap<(NoticeKind, String), Instant>,
}

impl NoticeThrottle {
    /// Whether to show `kind` for `app` now; remembers it if so.
    pub fn should_show(&mut self, kind: NoticeKind, app: &str, now: Instant) -> bool {
        let key = (kind, app.to_lowercase());
        match self.shown.get(&key) {
            Some(last) if now.saturating_duration_since(*last) < REPEAT_AFTER => false,
            _ => {
                self.shown.insert(key, now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [NoticeKind; 9] = [
        NoticeKind::CantRead,
        NoticeKind::NothingSelected,
        NoticeKind::PasswordField,
        NoticeKind::BlockedApp,
        NoticeKind::ClipboardSync,
        NoticeKind::OllamaNotRunning,
        NoticeKind::ModelMissing,
        NoticeKind::HookFailed,
        NoticeKind::UiaUnavailable,
    ];

    #[test]
    fn every_notice_has_a_title_and_detail() {
        for kind in ALL {
            let n = notice(kind, Context::default());
            assert!(!n.title.trim().is_empty(), "{kind:?}");
            assert!(!n.detail.trim().is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn app_name_is_shown() {
        let cx = Context { app: Some("keepass.exe"), ..Context::default() };
        assert_eq!(notice(NoticeKind::BlockedApp, cx).title, "Turned off for keepass.exe");
        let cx = Context { app: Some("Outlook"), ..Context::default() };
        assert!(notice(NoticeKind::CantRead, cx).detail.starts_with("Outlook "));
        // Without a name it still reads well.
        assert!(notice(NoticeKind::CantRead, Context::default()).detail.starts_with("This app "));
    }

    #[test]
    fn reason_is_shown() {
        let cx = Context { app: Some("Microsoft Edge"), reason: Some("the app didn't copy anything"), ..Context::default() };
        assert_eq!(
            notice(NoticeKind::NothingSelected, cx).detail,
            "Microsoft Edge didn't share the selection, and copying didn't work: the app didn't copy anything. Select the text again."
        );
        assert!(notice(NoticeKind::CantRead, cx).detail.ends_with("copying didn't work: the app didn't copy anything."));
    }

    #[test]
    fn missing_model_says_how_to_install_it() {
        let cx = Context { model: Some("qwen3.5:4b"), ..Context::default() };
        assert_eq!(notice(NoticeKind::ModelMissing, cx).detail, "Run: ollama pull qwen3.5:4b");
    }

    #[test]
    fn actions() {
        assert_eq!(notice(NoticeKind::BlockedApp, Context::default()).action, Some(NoticeAction::OpenSettings));
        assert_eq!(notice(NoticeKind::OllamaNotRunning, Context::default()).action, Some(NoticeAction::Retry));
        let startup = Context { at_startup: true, ..Context::default() };
        assert_eq!(notice(NoticeKind::OllamaNotRunning, startup).action, Some(NoticeAction::OpenSettings));
        assert_eq!(notice(NoticeKind::PasswordField, Context::default()).action, None);
    }

    #[test]
    fn startup_reason_is_shown() {
        let cx = Context { reason: Some("SetWindowsHookExW failed"), ..Context::default() };
        assert_eq!(notice(NoticeKind::HookFailed, cx).detail, "SetWindowsHookExW failed. Restart Screen Buddy.");
    }

    #[test]
    fn the_same_notice_twice_at_once_is_shown_once() {
        let mut throttle = NoticeThrottle::default();
        let t0 = Instant::now();
        assert!(throttle.should_show(NoticeKind::CantRead, "Outlook", t0));
        assert!(!throttle.should_show(NoticeKind::CantRead, "outlook", t0 + Duration::from_millis(500)));
        // Another app or another kind is shown.
        assert!(throttle.should_show(NoticeKind::CantRead, "Notepad", t0));
        assert!(throttle.should_show(NoticeKind::BlockedApp, "Outlook", t0));
    }

    #[test]
    fn every_later_selection_shows_it_again() {
        let mut throttle = NoticeThrottle::default();
        let t0 = Instant::now();
        assert!(throttle.should_show(NoticeKind::NothingSelected, "Edge", t0));
        assert!(throttle.should_show(NoticeKind::NothingSelected, "Edge", t0 + REPEAT_AFTER));
        assert!(throttle.should_show(NoticeKind::NothingSelected, "Edge", t0 + REPEAT_AFTER * 3));
    }

    #[test]
    fn notice_json_shape() {
        let n = notice(NoticeKind::BlockedApp, Context { app: Some("a.exe"), ..Context::default() });
        assert!(n.detail.contains("block list"));
        let value = serde_json::to_value(&n).unwrap();
        assert_eq!(value["kind"], "blocked_app");
        assert_eq!(value["action"], "open_settings");
        assert_eq!(value["title"], "Turned off for a.exe");
    }
}
