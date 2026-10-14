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

/// Longest greeting shown; a model that rambles is cut here.
const MAX_GREETING_CHARS: usize = 120;
const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// The part of the day for the local `hour` (0-23).
pub fn part_of_day(hour: u16) -> &'static str {
    match hour {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=21 => "evening",
        _ => "night",
    }
}

/// Startup greetings that need no model: used when Ollama isn't running, the model is missing,
/// or the model is too slow to write one. `{name}` is the user's name.
pub const SAVED_GREETINGS: [&str; 4] = [
    "Hey {name}! 👋 I'm ready. Select any word and I'll explain it.",
    "Welcome back, {name}! 🤖 Got a tricky word? Just select it.",
    "Hi {name}! ✨ Your word buddy is here. Select anything you want explained.",
    "Hello {name}! 📚 Select a word or a sentence, and I'll make it simple.",
];

/// One of the `SAVED_GREETINGS` (`index` wraps around), with the user's `name`, or without a
/// name when it is empty.
pub fn saved_greeting(index: usize, name: &str) -> String {
    let template = SAVED_GREETINGS[index % SAVED_GREETINGS.len()];
    let name = name.trim();
    if name.is_empty() {
        template.replace(", {name}", "").replace(" {name}", "")
    } else {
        template.replace("{name}", name)
    }
}

/// A different mood each launch, so the model doesn't settle into one stock greeting.
pub const GREETING_MOODS: [&str; 8] =
    ["playful", "cosy", "energetic", "curious", "witty", "gentle", "adventurous", "sunny"];

/// The system prompt and user message asking the model for a fresh greeting. Only the name,
/// the weekday and the part of the day are sent, to the local model. `weekday`: 0 = Sunday.
/// `mood`: an index into `GREETING_MOODS` (any number; it wraps around).
pub fn greeting_prompt(name: &str, hour: u16, weekday: u16, mood: usize) -> (String, String) {
    let system = "You are Screen Buddy, a cute, cheerful little robot that lives on the user's screen and \
                  explains words. Write one short greeting, at most 22 words, with exactly one fitting emoji. \
                  It must start with the user's name when you are given one, greet them for the time of day, \
                  and end by offering your help in fresh words. Never reuse a stock phrase: make every \
                  greeting and every offer of help unique. Reply with the greeting only, no quotes."
        .to_owned();
    let day = WEEKDAYS.get(usize::from(weekday)).copied().unwrap_or("today");
    let mood = GREETING_MOODS[mood % GREETING_MOODS.len()];
    let when = format!("It is {day} {}. Mood: {mood}.", part_of_day(hour));
    let name = name.trim();
    let user = if name.is_empty() {
        format!("The user's name is unknown, so don't use a name. {when}")
    } else {
        format!("The user's name is {name}. Start with \"{name}\". {when}")
    };
    (system, user)
}

/// Makes sure the greeting has the user's name: small models sometimes leave it out.
pub fn ensure_name(greeting: String, name: &str) -> String {
    let name = name.trim();
    if name.is_empty() || greeting.to_lowercase().contains(&name.to_lowercase()) {
        greeting
    } else {
        format!("Hey {name}! {greeting}")
    }
}

/// The model's greeting, tidied: the first non-empty line, without a "Greeting:" label or
/// quotes, cut to `MAX_GREETING_CHARS`. `None` if nothing usable is left.
pub fn clean_greeting(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = match line.split_once(':') {
        Some((label, rest)) if label.trim().eq_ignore_ascii_case("greeting") => rest.trim(),
        _ => line,
    };
    let line = line.trim_matches(|c| matches!(c, '"' | '“' | '”' | '\'' | '*' | '`')).trim();
    if line.is_empty() {
        return None;
    }
    Some(match line.char_indices().nth(MAX_GREETING_CHARS) {
        Some((cut, _)) => format!("{}…", line[..cut].trim_end()),
        None => line.to_owned(),
    })
}

/// How long the greeting bubble stays: long enough to read, a little longer for longer text.
pub fn greeting_duration(chars: usize) -> Duration {
    const BASE_MS: u64 = 3500;
    const MAX_MS: u64 = 6000;
    let extra = chars.saturating_sub(40) as u64 * 40;
    Duration::from_millis((BASE_MS + extra).min(MAX_MS))
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
    fn part_of_day_follows_the_hour() {
        for (hour, expected) in [
            (0, "night"),
            (4, "night"),
            (5, "morning"),
            (11, "morning"),
            (12, "afternoon"),
            (16, "afternoon"),
            (17, "evening"),
            (21, "evening"),
            (22, "night"),
        ] {
            assert_eq!(part_of_day(hour), expected, "hour {hour}");
        }
    }

    #[test]
    fn saved_greeting_uses_the_name() {
        assert_eq!(saved_greeting(0, " Rajnesh "), "Hey Rajnesh! 👋 I'm ready. Select any word and I'll explain it.");
        assert_eq!(saved_greeting(1, "Rajnesh"), "Welcome back, Rajnesh! 🤖 Got a tricky word? Just select it.");
        for i in 0..SAVED_GREETINGS.len() {
            assert!(saved_greeting(i, "Rajnesh").contains("Rajnesh"), "greeting {i}");
        }
    }

    #[test]
    fn saved_greeting_without_a_name_reads_well() {
        assert_eq!(saved_greeting(0, ""), "Hey! 👋 I'm ready. Select any word and I'll explain it.");
        assert_eq!(saved_greeting(1, "  "), "Welcome back! 🤖 Got a tricky word? Just select it.");
        for i in 0..SAVED_GREETINGS.len() {
            let text = saved_greeting(i, "");
            assert!(!text.contains("{name}") && !text.contains(" !") && !text.contains(",!"), "{text}");
        }
    }

    #[test]
    fn saved_greetings_are_all_different_and_wrap_around() {
        let all: std::collections::HashSet<String> = (0..SAVED_GREETINGS.len()).map(|i| saved_greeting(i, "R")).collect();
        assert_eq!(all.len(), SAVED_GREETINGS.len());
        assert_eq!(saved_greeting(2 + SAVED_GREETINGS.len(), "R"), saved_greeting(2, "R"));
    }

    #[test]
    fn greeting_prompt_has_the_name_time_mood_and_help() {
        let (system, user) = greeting_prompt("Rajnesh", 14, 2, 1);
        assert!(system.contains("offering your help in fresh words"));
        assert!(system.contains("must start with the user's name"));
        assert_eq!(user, "The user's name is Rajnesh. Start with \"Rajnesh\". It is Tuesday afternoon. Mood: cosy.");
    }

    #[test]
    fn greeting_prompt_without_a_name_asks_for_none() {
        let (_, user) = greeting_prompt("  ", 23, 0, 0);
        assert_eq!(user, "The user's name is unknown, so don't use a name. It is Sunday night. Mood: playful.");
    }

    #[test]
    fn mood_wraps_around() {
        let (_, a) = greeting_prompt("R", 9, 1, 3);
        let (_, b) = greeting_prompt("R", 9, 1, 3 + GREETING_MOODS.len());
        assert_eq!(a, b);
    }

    #[test]
    fn name_is_added_when_the_model_leaves_it_out() {
        assert_eq!(ensure_name("Good morning! ☀️".into(), "Rajnesh"), "Hey Rajnesh! Good morning! ☀️");
        assert_eq!(ensure_name("rajnesh, good morning!".into(), "Rajnesh"), "rajnesh, good morning!");
        assert_eq!(ensure_name("Good morning!".into(), ""), "Good morning!");
    }

    #[test]
    fn model_greeting_is_tidied() {
        assert_eq!(clean_greeting("\n  \"Hey Rajnesh! 🌟 Ready to learn?\"  \n"), Some("Hey Rajnesh! 🌟 Ready to learn?".into()));
        assert_eq!(clean_greeting("Greeting: Hi Rajnesh 👋"), Some("Hi Rajnesh 👋".into()));
        assert_eq!(clean_greeting("Hello there 😊\nThis second line is dropped."), Some("Hello there 😊".into()));
        assert_eq!(clean_greeting("  \n\"\"\n"), None);
        let long = "a".repeat(300);
        assert_eq!(clean_greeting(&long).unwrap().chars().count(), MAX_GREETING_CHARS + 1);
    }

    #[test]
    fn longer_greetings_stay_a_little_longer() {
        assert_eq!(greeting_duration(20), Duration::from_millis(3500));
        assert_eq!(greeting_duration(60), Duration::from_millis(4300));
        assert_eq!(greeting_duration(500), Duration::from_millis(6000));
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
