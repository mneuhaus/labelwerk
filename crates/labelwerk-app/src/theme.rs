//! "Werkbank" look: a cutting mat as the work surface, signal yellow for the one action that matters,
//! Barlow (signage grotesque) for the interface and IBM Plex Mono for measurements.

use std::borrow::Cow;
use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig};
use gpui_kit::*;

pub const UI_FONT: &str = "Barlow";
pub const DISPLAY_FONT: &str = "Barlow Semi Condensed";
pub const MONO_FONT: &str = "IBM Plex Mono";

/// Cutting mat, the same in light and dark mode (it is an object, not chrome).
pub const MAT: u32 = 0x22403a;
/// 1 cm grid and 5 cm grid on the mat, and the measurement lines drawn on it.
pub const MAT_GRID: u32 = 0xffffff12;
pub const MAT_GRID_MAJOR: u32 = 0xffffff26;
pub const MAT_INK: u32 = 0xe8f0ebcc;

pub fn load_fonts(cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Barlow-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Barlow-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Barlow-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/BarlowSemiCondensed-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/BarlowSemiCondensed-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/BarlowSemiCondensed-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf")),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("could not load the interface fonts: {e:#}");
    }
}

/// (field, light, dark)
const COLORS: &[(&str, &str, &str)] = &[
    ("background", "#f3f4f1", "#0e1412"),
    ("foreground", "#17201c", "#e3e9e5"),
    ("border", "#d9ded8", "#243029"),
    ("input", "#cfd6cf", "#2d3a33"),
    ("muted", "#e9ece7", "#161f1b"),
    ("muted_foreground", "#5c6a63", "#8d9b94"),
    ("sidebar", "#f9faf8", "#111916"),
    ("sidebar_border", "#d9ded8", "#243029"),
    ("sidebar_foreground", "#17201c", "#e3e9e5"),
    ("secondary", "#e9ece7", "#1a2420"),
    ("secondary_hover", "#dfe4de", "#212d28"),
    ("secondary_active", "#d4dbd3", "#283630"),
    ("secondary_foreground", "#17201c", "#e3e9e5"),
    ("accent", "#e4e9e3", "#1c2722"),
    ("accent_foreground", "#17201c", "#e3e9e5"),
    ("primary", "#f2c230", "#f2c230"),
    ("primary_hover", "#e7b522", "#f5cd52"),
    ("primary_active", "#d6a515", "#dcae22"),
    ("primary_foreground", "#17201c", "#121815"),
    ("success", "#2f8a57", "#4cb57a"),
    ("success_foreground", "#ffffff", "#0e1412"),
    ("warning", "#c98216", "#e3a43c"),
    ("warning_foreground", "#17201c", "#0e1412"),
    ("danger", "#c63d2f", "#e3604f"),
    ("danger_foreground", "#ffffff", "#ffffff"),
    ("ring", "#2f6b57", "#f2c230"),
    ("selection", "#cfe3d9", "#2a4a3e"),
    ("popover", "#ffffff", "#141c19"),
    ("popover_foreground", "#17201c", "#e3e9e5"),
    ("list_hover", "#e9ece7", "#1c2722"),
    ("switch", "#cdd4cd", "#2d3a33"),
    ("title_bar", "#f9faf8", "#111916"),
    ("title_bar_border", "#d9ded8", "#243029"),
];

fn apply(config: &Rc<ThemeConfig>, dark: bool) -> Rc<ThemeConfig> {
    let mut c = (**config).clone();
    let value = |name: &str| COLORS.iter().find(|(k, _, _)| *k == name).map(|(_, l, d)| if dark { *d } else { *l });
    macro_rules! set {
        ($($field:ident),*) => { $( if let Some(v) = value(stringify!($field)) { c.colors.$field = Some(v.to_string().into()); } )* };
    }
    set!(
        background, foreground, border, input, muted, muted_foreground, sidebar, sidebar_border, sidebar_foreground,
        secondary, secondary_hover, secondary_active, secondary_foreground, accent, accent_foreground, primary,
        primary_hover, primary_active, primary_foreground, success, success_foreground, warning, warning_foreground,
        danger, danger_foreground, ring, selection, popover, popover_foreground, list_hover, switch, title_bar,
        title_bar_border
    );
    c.colors.button_primary = c.colors.primary.clone();
    c.colors.button_primary_hover = c.colors.primary_hover.clone();
    c.colors.button_primary_active = c.colors.primary_active.clone();
    c.colors.button_primary_foreground = c.colors.primary_foreground.clone();
    c.font_family = Some(UI_FONT.into());
    c.radius = Some(6);
    c.radius_lg = Some(10);
    Rc::new(c)
}

/// Put the palette into gpui-kit's light and dark themes (before `Theme::change`).
pub fn register(cx: &mut App) {
    let t = Theme::global_mut(cx);
    t.light_theme = apply(&t.light_theme, false);
    t.dark_theme = apply(&t.dark_theme, true);
}
