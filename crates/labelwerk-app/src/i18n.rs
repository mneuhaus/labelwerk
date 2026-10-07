//! Decide once whether the UI shows English or German text.
//!
//! `LABELWERK_LANG=en|de` overrides; otherwise the system locale decides: German when it starts
//! with "de", English otherwise.

use std::sync::OnceLock;

static GERMAN: OnceLock<bool> = OnceLock::new();

/// True once decided: show German text instead of English.
pub fn german() -> bool {
    *GERMAN.get_or_init(|| match std::env::var("LABELWERK_LANG").as_deref() {
        Ok("de") => true,
        Ok("en") => false,
        _ => sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("de")),
    })
}

/// Pick the English or German arm depending on the detected UI language. For text built with
/// `format!`, use `i18n::german()` directly in an `if`/`else` instead (see printer.rs, app.rs).
#[macro_export]
macro_rules! tr {
    ($en:literal, $de:literal) => {
        if $crate::i18n::german() { $de } else { $en }
    };
}
