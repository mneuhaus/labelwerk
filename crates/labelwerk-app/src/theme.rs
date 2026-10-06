//! Look: a neutral work surface with a true-scale centimetre grid, signal yellow for the one action that
//! matters, Barlow (signage grotesque) for the interface and IBM Plex Mono for measurements.

use std::borrow::Cow;
use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig};
use gpui_kit::*;

pub const UI_FONT: &str = "Barlow";
pub const DISPLAY_FONT: &str = "Barlow Semi Condensed";
pub const MONO_FONT: &str = "IBM Plex Mono";

/// The work surface the label lies on, with a centimetre grid at true scale.
#[derive(Debug, Clone, Copy)]
pub struct Canvas {
    pub bg: u32,
    /// 1 cm and 5 cm grid lines (RGBA)
    pub grid: u32,
    pub grid_major: u32,
    /// measurements and captions drawn on the surface (RGBA)
    pub ink: u32,
    /// edge around the label so white paper stands out on a light surface (RGBA)
    pub edge: u32,
}

const GRAPHITE: Canvas = Canvas { bg: 0x232529, grid: 0xffffff0f, grid_major: 0xffffff21, ink: 0xe6e8ebcc, edge: 0x00000000 };
const DRAFTING: Canvas = Canvas { bg: 0xe8e9ec, grid: 0x0000000c, grid_major: 0x0000001a, ink: 0x2b2e33cc, edge: 0x00000024 };
const MAT: Canvas = Canvas { bg: 0x22403a, grid: 0xffffff12, grid_major: 0xffffff26, ink: 0xe8f0ebcc, edge: 0x00000000 };

/// Graphite in dark mode, drafting grey in light mode; `LABELWERK_CANVAS=graphit|hell|matte` forces one.
pub fn canvas(dark: bool) -> Canvas {
    match std::env::var("LABELWERK_CANVAS").as_deref() {
        Ok("graphit") => GRAPHITE,
        Ok("hell") => DRAFTING,
        Ok("matte") => MAT,
        _ if dark => GRAPHITE,
        _ => DRAFTING,
    }
}

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
    ("background", "#f5f5f6", "#121315"),
    ("foreground", "#18191b", "#e7e8ea"),
    ("border", "#e0e1e4", "#26282c"),
    ("input", "#d6d8dc", "#30333a"),
    ("muted", "#ececee", "#1a1c1f"),
    ("muted_foreground", "#64676d", "#93969c"),
    ("sidebar", "#fbfbfc", "#16171a"),
    ("sidebar_border", "#e0e1e4", "#26282c"),
    ("sidebar_foreground", "#18191b", "#e7e8ea"),
    ("secondary", "#ececee", "#1f2124"),
    ("secondary_hover", "#e3e4e7", "#26282c"),
    ("secondary_active", "#d9dade", "#2e3035"),
    ("secondary_foreground", "#18191b", "#e7e8ea"),
    ("accent", "#ebecee", "#202226"),
    ("accent_foreground", "#18191b", "#e7e8ea"),
    ("primary", "#f2c230", "#f2c230"),
    ("primary_hover", "#e7b522", "#f5cd52"),
    ("primary_active", "#d6a515", "#dcae22"),
    ("primary_foreground", "#18191b", "#141414"),
    ("success", "#2f8a57", "#4cb57a"),
    ("success_foreground", "#ffffff", "#121315"),
    ("warning", "#c98216", "#e3a43c"),
    ("warning_foreground", "#18191b", "#121315"),
    ("danger", "#c63d2f", "#e3604f"),
    ("danger_foreground", "#ffffff", "#ffffff"),
    ("ring", "#18191b", "#f2c230"),
    ("selection", "#e6e7ea", "#2a2c31"),
    ("popover", "#ffffff", "#18191c"),
    ("popover_foreground", "#18191b", "#e7e8ea"),
    ("list_hover", "#ececee", "#202226"),
    ("switch", "#d4d6da", "#33363c"),
    ("title_bar", "#fbfbfc", "#16171a"),
    ("title_bar_border", "#e0e1e4", "#26282c"),
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
