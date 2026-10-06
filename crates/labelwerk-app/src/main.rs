//! Labelwerk: labels for the Brother QL-1100, without the clutter.
//!
//! Environment: `LABELWERK_STATE=<file>` keeps state somewhere else (tests, screenshots),
//! `LABELWERK_THEME=light|dark` overrides the system appearance.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod printer;
mod store;
mod tape;
mod theme;

use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Printer, RotateCw, Bold, Italic, RefreshCw, TextAlignStart, TextAlignCenter, TextAlignEnd, Tag, LoaderCircle,
        Unplug, CircleAlert, CircleCheck, Heading, QrCode, Square, Info
    ]
);

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(b) = ExtraIcons.load(path)? {
            return Ok(Some(b));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut v = gpui_kit::assets::Assets.list(path)?;
        v.extend(ExtraIcons.list(path)?);
        v.sort();
        v.dedup();
        Ok(v)
    }
}

gpui_kit::actions!(window_actions, [Quit, CloseWindow, Hide]);

fn menus(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &CloseWindow, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("alt-f4", Quit, None),
    ]);
    cx.set_menus([Menu::new("Labelwerk").items([
        MenuItem::action("Labelwerk ausblenden", Hide),
        MenuItem::separator(),
        MenuItem::action("Labelwerk beenden", Quit),
    ])]);
}

fn apply_theme(window: Option<&mut Window>, cx: &mut App) {
    match std::env::var("LABELWERK_THEME").as_deref() {
        Ok("dark") => Theme::change(ThemeMode::Dark, window, cx),
        Ok("light") => Theme::change(ThemeMode::Light, window, cx),
        _ => Theme::sync_system_appearance(window, cx),
    }
}

fn main() {
    gpui_kit::application().with_assets(AppAssets).run(|cx| {
        gpui_kit::init(cx);
        theme::load_fonts(cx);
        theme::register(cx);
        apply_theme(None, cx);
        menus(cx);
        app::bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(1240.), px(800.)), cx);
        let opened = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Labelwerk".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(18.))),
                }),
                window_min_size: Some(size(px(980.), px(640.))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                window.observe_window_appearance(|window, cx| apply_theme(Some(window), cx)).detach();
                window.on_window_should_close(cx, |_, cx| {
                    cx.defer(|cx| cx.quit());
                    true
                });
                let view = cx.new(|cx| app::LabelApp::new(window, cx));
                let text = view.read(cx).text_input();
                text.update(cx, |t, cx| t.focus(window, cx));
                view
            },
        );
        if let Err(e) = opened {
            eprintln!("could not open the window: {e:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}
