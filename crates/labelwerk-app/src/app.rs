//! The one window: editor on the left, exact print preview on the right, print button below.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::searchable_list::{SearchableGroup, SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IndexPath, Selectable as _, Sizable, WindowExt};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use labelwerk_core::media::dots_to_mm;
use labelwerk_core::bitmap::encode_png_rgba;
use labelwerk_core::render::{PreviewStyle, preview_png};
use labelwerk_core::model::Support;
use labelwerk_core::{Align, Direction, Family, Kind, Label, Media, Model, PrintOptions, Rendered, Renderer, models};

use crate::printer::{self, PrinterState};
use crate::{i18n, tape, theme, tr};
use crate::store::{self, HistoryEntry, Saved};

const INSPECTOR_W: f32 = 340.;
const TITLE_H: f32 = 52.;
const BAR_H: f32 = 72.;
/// Never draw a label larger than this many screen points per millimetre (about 3x real size).
const MAX_PT_PER_MM: f32 = 11.0;

gpui_kit::actions!(labelwerk, [PrintLabel, RotateLabel]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-p", PrintLabel, None),
        KeyBinding::new("ctrl-p", PrintLabel, None),
        KeyBinding::new("cmd-r", RotateLabel, None),
        KeyBinding::new("ctrl-r", RotateLabel, None),
    ]);
}

/// Display name of a medium, in the UI language.
pub fn media_name(m: &Media) -> String {
    let (w, l) = m.nominal_mm();
    let (w, l) = (mm(w), mm(l));
    match (m.family, m.kind) {
        (Family::Pt, _) if m.is_tube() => format!("{w} mm {}", tr!("heat-shrink tube", "Schrumpfschlauch")),
        (Family::Pt, _) => format!("{w} mm {}", tr!("tape", "Band")),
        (_, Kind::Continuous) => format!("{w} mm {}", tr!("continuous tape", "Endlosband")),
        (_, Kind::DieCut) => format!("{w} × {l} mm"),
        (_, Kind::Round) => format!("Ø {w} mm {}", tr!("round", "rund")),
    }
}

/// A millimetre value for display: German uses a decimal comma, English a decimal point.
fn mm(v: f32) -> String {
    let s = format!("{v:.1}");
    let s = s.trim_end_matches(".0");
    if i18n::german() { s.replace('.', ",") } else { s.to_string() }
}

#[derive(Clone)]
struct MediaItem {
    key: SharedString,
    title: SharedString,
    loaded: bool,
}

impl SearchableListItem for MediaItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_2()
            .child(self.title.clone())
            .when(self.loaded, |d| d.child(div().text_xs().text_color(cx.theme().success).child(tr!("loaded", "eingelegt"))))
    }

    fn value(&self) -> &SharedString {
        &self.key
    }
}

fn media_groups(model: &'static Model, loaded: Option<&Media>) -> SearchableVec<SearchableGroup<MediaItem>> {
    let group = |title: &str, filter: &dyn Fn(&Media) -> bool| {
        SearchableGroup::new(title.to_string()).items(model.media.iter().filter(|m| filter(m)).map(|m| MediaItem {
            key: m.key().into(),
            title: media_name(m).into(),
            loaded: loaded.is_some_and(|l| l.id == m.id),
        }))
    };
    let groups = match model.family {
        Family::Pt => vec![
            group(tr!("Tapes", "Bänder"), &|m| !m.is_tube()),
            group(tr!("Heat-shrink tube", "Schrumpfschlauch"), &|m| m.is_tube()),
        ],
        Family::Ql => vec![
            group(tr!("Continuous tape", "Endlosband"), &|m| m.kind == Kind::Continuous),
            group(tr!("Labels", "Etiketten"), &|m| m.kind == Kind::DieCut),
            group(tr!("Round", "Rund"), &|m| m.kind == Kind::Round),
        ],
    };
    SearchableVec::new(groups.into_iter().filter(|g| !g.items.is_empty()).collect::<Vec<_>>())
}

fn model_names() -> Vec<SharedString> {
    models().iter().map(|m| SharedString::from(m.name.clone())).collect()
}

pub struct LabelApp {
    renderer: Renderer,
    label: Label,
    model: &'static Model,
    media: &'static Media,
    follow_printer: bool,
    copies: u32,
    history: Vec<HistoryEntry>,
    printer: PrinterState,
    /// Last model and roll the printer reported; the editor follows changes of them, not every poll.
    seen_model: Option<&'static Model>,
    seen_media: Option<&'static Media>,
    printing: bool,
    rendered: Option<Rendered>,
    preview: Option<Arc<Image>>,
    /// Show the printer's dots instead of the smooth preview.
    show_dots: bool,
    /// Pixels per printer dot the smooth preview was drawn with.
    preview_k: u32,
    /// Mini previews of `history` (image, width / height).
    thumbs: Vec<(Arc<Image>, f32)>,
    text: Entity<TextareaState>,
    font: Entity<SelectState<SearchableVec<SharedString>>>,
    model_select: Entity<SelectState<SearchableVec<SharedString>>>,
    media_select: Entity<SelectState<SearchableVec<SearchableGroup<MediaItem>>>>,
    size: Entity<InputState>,
    length: Entity<InputState>,
    padding: Entity<InputState>,
    qr_text: Entity<InputState>,
    copies_input: Entity<InputState>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl LabelApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = store::load();
        let renderer = Renderer::new();
        let mut label = saved.label.clone();
        if label.font.is_empty() {
            label.font = renderer.default_family();
        }
        let model = Model::by_name(&saved.model).unwrap_or_else(Model::default_model);
        let media = model.media_by_key(&saved.media).unwrap_or(&model.media[0]);

        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 10)
                .placeholder(tr!("Label text", "Text des Etiketts"))
                .default_value(label.text.clone())
        });
        let families: Vec<SharedString> = renderer.families().iter().map(|f| SharedString::from(f.clone())).collect();
        let font_ix = families.iter().position(|f| f.as_ref() == label.font).map(IndexPath::new);
        let font = cx.new(|cx| SelectState::new(SearchableVec::new(families), font_ix, window, cx).searchable(true));
        let model_ix = models().iter().position(|m| m.name == model.name).map(IndexPath::new);
        let model_select = cx.new(|cx| SelectState::new(SearchableVec::new(model_names()), model_ix, window, cx).searchable(true));
        let media_select = cx.new(|cx| {
            let mut s = SelectState::new(media_groups(model, None), None, window, cx);
            s.set_selected_value(&SharedString::from(media.key()), window, cx);
            s
        });
        let number = |value: Option<f32>, min: f64, max: f64, step: f64, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let s = InputState::new(window, cx).min(min).max(max).step(step);
                match value {
                    Some(v) => s.default_value(mm(v)),
                    None => s,
                }
            })
        };
        let size = number(label.size_pt, 4.0, 400.0, 1.0, window, cx);
        let length = number(label.length_mm, 10.0, 1000.0, 5.0, window, cx);
        let padding = number(Some(label.padding_mm), 0.0, 20.0, 0.5, window, cx);
        let copies_input = number(Some(saved.copies.max(1) as f32), 1.0, 99.0, 1.0, window, cx);
        let qr_text = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr!("Content, blank = label text", "Inhalt, leer = Etikettentext"))
                .default_value(label.qr_content.clone())
        });

        let mut subs = vec![
            cx.subscribe_in(&text, window, |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.label.text = state.read(cx).value().to_string();
                    this.changed(window, cx);
                }
            }),
            cx.subscribe_in(&font, window, |this, _, ev: &SelectEvent<SearchableVec<SharedString>>, window, cx| {
                let SelectEvent::Confirm(Some(family)) = ev else { return };
                this.label.font = family.to_string();
                this.changed(window, cx);
            }),
            cx.subscribe_in(&media_select, window, |this, _, ev: &SelectEvent<SearchableVec<SearchableGroup<MediaItem>>>, window, cx| {
                let SelectEvent::Confirm(Some(key)) = ev else { return };
                if let Some(m) = this.model.media_by_key(key) {
                    this.set_media(m, window, cx);
                }
            }),
            cx.subscribe_in(&model_select, window, |this, _, ev: &SelectEvent<SearchableVec<SharedString>>, window, cx| {
                let SelectEvent::Confirm(Some(name)) = ev else { return };
                if let Some(m) = Model::by_name(name) {
                    this.set_model(m, None, window, cx);
                }
            }),
            cx.subscribe_in(&qr_text, window, |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.label.qr_content = state.read(cx).value().to_string();
                    this.changed(window, cx);
                }
            }),
        ];
        let number_field = |input: &Entity<InputState>, apply: fn(&mut LabelApp, Option<f32>), window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(input, window, move |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let raw = state.read(cx).value().trim().replace(',', ".");
                    apply(this, raw.parse::<f32>().ok().filter(|v| v.is_finite() && *v > 0.0));
                    this.changed(window, cx);
                }
            })
        };
        subs.push(number_field(&size, |t, v| t.label.size_pt = v.map(|v| v.clamp(4.0, 400.0)), window, cx));
        subs.push(number_field(&length, |t, v| t.label.length_mm = v.map(|v| v.clamp(10.0, 1000.0)), window, cx));
        subs.push(number_field(&padding, |t, v| t.label.padding_mm = v.unwrap_or(0.0).clamp(0.0, 20.0), window, cx));
        subs.push(number_field(&copies_input, |t, v| t.copies = v.map(|v| v.round().clamp(1.0, 99.0) as u32).unwrap_or(1), window, cx));

        let mut this = Self {
            renderer,
            label,
            model,
            media,
            follow_printer: saved.follow_printer,
            copies: saved.copies.max(1),
            history: saved.history,
            printer: PrinterState::Searching,
            seen_model: None,
            seen_media: None,
            printing: false,
            rendered: None,
            preview: None,
            show_dots: false,
            preview_k: 0,
            thumbs: Vec::new(),
            text,
            font,
            model_select,
            media_select,
            size,
            length,
            padding,
            qr_text,
            copies_input,
            focus: cx.focus_handle(),
            _subscriptions: subs,
        };
        this.rerender(window, cx);
        this.refresh_thumbs();
        // the smooth preview is drawn for the size it is shown at
        this._subscriptions.push(cx.observe_window_bounds(window, |this, window, cx| {
            let wanted = this.rendered.as_ref().map(|r| preview_k(&r.geometry, window));
            if !this.show_dots && wanted.is_some_and(|k| k != this.preview_k) {
                this.rerender(window, cx);
            }
        }));
        this.start_polling(window, cx);
        this
    }

    pub fn text_input(&self) -> Entity<TextareaState> {
        self.text.clone()
    }

    fn saved(&self) -> Saved {
        Saved {
            label: self.label.clone(),
            model: self.model.name.clone(),
            media: self.media.key(),
            follow_printer: self.follow_printer,
            copies: self.copies,
            history: self.history.clone(),
        }
    }

    /// Something about the label changed: re-render and remember it.
    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rerender(window, cx);
        store::save(&self.saved());
    }

    fn rerender(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rendered = self.renderer.render(&self.label, self.model, self.media);
        let look = self.look();
        let style = PreviewStyle { paper: look.tape, ink: look.ink, outline: Some(mix(look.tape, look.ink, 0.10)) };
        let png = if self.show_dots {
            preview_png(&rendered, &style)
        } else {
            let k = preview_k(&rendered.geometry, window);
            self.preview_k = k;
            let (w, h, rgba) = self.renderer.preview_smooth(&rendered, k, &style);
            encode_png_rgba(w, h, &rgba)
        };
        self.preview = Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)));
        let auto = |v: Option<f32>, unit: &str| v.map(|v| format!("Auto ({} {unit})", mm(v))).unwrap_or_else(|| "Auto".into());
        let size_hint = auto(rendered.font_pt, "pt");
        let length_hint = auto(Some(dots_to_mm(rendered.geometry.along_dots(), rendered.geometry.dpi)), "mm");
        self.size.update(cx, |s, cx| s.set_placeholder(size_hint, window, cx));
        self.length.update(cx, |s, cx| s.set_placeholder(length_hint, window, cx));
        self.rendered = Some(rendered);
        cx.notify();
    }

    /// Switch to another printer model, keeping the label; picks `media` or the model's first medium.
    fn set_model(&mut self, model: &'static Model, media: Option<&'static Media>, window: &mut Window, cx: &mut Context<Self>) {
        self.model = model;
        // the same size on the new model if it has one (62 mm stays 62 mm), else its first medium
        let media = media.or_else(|| model.media_by_key(&self.media.key())).unwrap_or(&model.media[0]);
        let loaded = self.seen_media.filter(|m| model.media.iter().any(|x| std::ptr::eq(x, *m)));
        self.media_select.update(cx, |s, cx| s.set_items(media_groups(model, loaded), window, cx));
        let name = SharedString::from(model.name.clone());
        self.model_select.update(cx, |s, cx| s.set_selected_value(&name, window, cx));
        self.media = media;
        self.label.direction = None;
        let key = SharedString::from(media.key());
        self.media_select.update(cx, |s, cx| s.set_selected_value(&key, window, cx));
        self.changed(window, cx);
    }

    /// Render the history entries small, as they were printed.
    fn refresh_thumbs(&mut self) {
        let mut thumbs = Vec::with_capacity(self.history.len());
        for entry in &self.history {
            let model = Model::by_name(&entry.model).unwrap_or(self.model);
            let media = model.media_by_key(&entry.media).unwrap_or(&model.media[0]);
            let r = self.renderer.render(&entry.label, model, media);
            let style = PreviewStyle { outline: None, ..PreviewStyle::default() };
            let aspect = r.geometry.label_w as f32 / r.geometry.label_h.max(1) as f32;
            thumbs.push((Arc::new(Image::from_bytes(ImageFormat::Png, preview_png(&r, &style))), aspect));
        }
        self.thumbs = thumbs;
    }

    fn set_media(&mut self, media: &'static Media, window: &mut Window, cx: &mut Context<Self>) {
        if self.media.id == media.id {
            return;
        }
        // A direction picked for one medium rarely suits another
        self.label.direction = None;
        self.media = media;
        self.changed(window, cx);
    }

    fn start_polling(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let state = cx.background_executor().spawn(async { printer::poll() }).await;
                let alive = this.update_in(cx, |this, window, cx| this.apply_printer(state, window, cx));
                if alive.is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(3)).await;
            }
        })
        .detach();
    }

    fn apply_printer(&mut self, state: PrinterState, window: &mut Window, cx: &mut Context<Self>) {
        if self.printing && state == PrinterState::Searching {
            return;
        }
        let model = state.model();
        let loaded = state.loaded_media();
        let name = |m: Option<&'static Model>| m.map(|m| m.name.as_str());
        let model_changed = name(model) != name(self.seen_model);
        let media_changed = loaded.map(|m| m.id) != self.seen_media.map(|m| m.id);
        if model_changed || media_changed {
            self.seen_model = model;
            self.seen_media = loaded;
            match model {
                // labels are always made for the printer that is connected
                Some(m) if m.name != self.model.name => self.set_model(m, loaded, window, cx),
                _ => {
                    let current = self.model;
                    self.media_select.update(cx, |s, cx| s.set_items(media_groups(current, loaded), window, cx));
                    match loaded {
                        Some(m) if self.follow_printer && m.id != self.media.id => self.set_media(m, window, cx),
                        _ => {}
                    }
                    let key = SharedString::from(self.media.key());
                    self.media_select.update(cx, |s, cx| s.set_selected_value(&key, window, cx));
                }
            }
        }
        if state != self.printer {
            let colors_changed = state.colors() != self.printer.colors();
            self.printer = state;
            if colors_changed {
                self.rerender(window, cx);
            }
            cx.notify();
        }
    }

    fn toggle_direction(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.label.direction = Some(self.label.direction_for(self.media).flipped());
        self.changed(window, cx);
    }

    fn use_loaded_media(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(m) = self.printer.loaded_media() {
            self.set_media(m, window, cx);
            let key = SharedString::from(m.key());
            self.media_select.update(cx, |s, cx| s.set_selected_value(&key, window, cx));
        }
    }

    fn load_history(&mut self, entry: HistoryEntry, window: &mut Window, cx: &mut Context<Self>) {
        let label = entry.label.clone();
        self.text.update(cx, |s, cx| s.set_value(label.text.clone(), window, cx));
        self.qr_text.update(cx, |s, cx| s.set_value(label.qr_content.clone(), window, cx));
        let opt = |v: Option<f32>| v.map(mm).unwrap_or_default();
        self.size.update(cx, |s, cx| s.set_value(opt(label.size_pt), window, cx));
        self.length.update(cx, |s, cx| s.set_value(opt(label.length_mm), window, cx));
        self.padding.update(cx, |s, cx| s.set_value(mm(label.padding_mm), window, cx));
        let family = SharedString::from(label.font.clone());
        self.font.update(cx, |s, cx| s.set_selected_value(&family, window, cx));
        self.label = label;
        let model = Model::by_name(&entry.model).unwrap_or(self.model);
        let media = model.media_by_key(&entry.media);
        self.set_model(model, media, window, cx);
    }

    /// Why printing is not possible right now, if it is not.
    fn blocker(&self) -> Option<String> {
        if self.printing {
            return Some(tr!("Printing…", "Druckt gerade …").to_string());
        }
        let empty = self.label.text.trim().is_empty() && !(self.label.qr && !self.label.qr_data().is_empty());
        if empty {
            return Some(tr!("The label is empty", "Das Etikett ist leer").to_string());
        }
        if self.model.protocol.support == Support::Unsupported {
            let suffix = tr!(
                "speaks a different protocol and is not supported",
                "spricht ein anderes Protokoll und wird nicht unterstützt"
            );
            return Some(format!("{} {suffix}", self.model.name));
        }
        match &self.printer {
            PrinterState::Searching => Some(tr!("Searching for printer…", "Suche Drucker …").to_string()),
            PrinterState::Missing { .. } => Some(if i18n::german() {
                format!("{} per USB anschließen und einschalten", self.model.name)
            } else {
                format!("Connect {} via USB and turn it on", self.model.name)
            }),
            PrinterState::Busy { queue: Some(_), .. } => None,
            PrinterState::Busy { message, .. } => {
                let prefix = tr!("Printer busy", "Drucker belegt");
                Some(format!("{prefix}: {message}"))
            }
            PrinterState::Problem { message, .. } => Some(message.clone()),
            PrinterState::Ready { model, .. } if model.name != self.model.name => Some(if i18n::german() {
                format!("Angeschlossen ist ein {}", model.name)
            } else {
                format!("A {} is connected", model.name)
            }),
            PrinterState::Ready { media: Some(loaded), .. } if loaded.id != self.media.id => Some(if i18n::german() {
                format!("Eingelegt ist {}", media_name(loaded))
            } else {
                format!("{} is loaded", media_name(loaded))
            }),
            PrinterState::Ready { .. } => None,
        }
    }

    fn print(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reason) = self.blocker() {
            window.push_notification(Notification::warning(reason).title(tr!("Cannot print", "Drucken nicht möglich")), cx);
            return;
        }
        let Some(rendered) = &self.rendered else { return };
        let pages = vec![rendered.page.clone(); self.copies as usize];
        let model = self.model;
        let media = self.media;
        let opts = PrintOptions::default();
        let queue = match &self.printer {
            PrinterState::Busy { queue: Some(q), .. } => Some(q.clone()),
            _ => None,
        };
        let entry = HistoryEntry { label: self.label.clone(), model: model.name.clone(), media: media.key() };
        let copies = self.copies;
        self.printing = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match queue {
                        Some(q) => printer::print_queue(q, model, media, pages, opts),
                        None => printer::print_usb(model, media, pages, opts),
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.printing = false;
                match result {
                    Ok(confirmed) => {
                        store::remember(&mut this.history, entry);
                        this.refresh_thumbs();
                        store::save(&this.saved());
                        let what = if i18n::german() {
                            if copies == 1 { "1 Etikett".to_string() } else { format!("{copies} Etiketten") }
                        } else if copies == 1 {
                            "1 label".to_string()
                        } else {
                            format!("{copies} labels")
                        };
                        let title = if confirmed { tr!("Printed", "Gedruckt") } else { tr!("Sent", "Gesendet") };
                        let joiner = tr!("on", "auf");
                        window.push_notification(
                            Notification::success(format!("{what} {joiner} {}", media_name(media))).title(title),
                            cx,
                        );
                    }
                    Err(e) => {
                        window.push_notification(
                            Notification::error(printer::localize(&format!("{e:#}"))).title(tr!("Print failed", "Druck fehlgeschlagen")),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // MARK: rendering

    /// How the loaded tape looks, when the printer reports this very medium; plain white otherwise.
    fn look(&self) -> tape::Look {
        match &self.printer {
            PrinterState::Ready { model, media: Some(m), colors } if model.name == self.model.name && m.id == self.media.id => {
                tape::look(self.model.family, *colors)
            }
            _ => tape::paper(),
        }
    }

    fn eyebrow(text: impl Into<SharedString>, cx: &Context<Self>) -> Div {
        div()
            .text_xs()
            .font_family(theme::MONO_FONT)
            .text_color(cx.theme().muted_foreground)
            .child(text.into().to_uppercase())
    }

    fn field(label: &'static str, input: impl IntoElement, cx: &Context<Self>) -> Div {
        h_flex()
            .gap_3()
            .items_center()
            .child(div().w(px(52.)).text_sm().text_color(cx.theme().muted_foreground).child(label))
            .child(div().flex_1().child(input))
    }

    fn render_titlebar(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let look = self.look();
        let (dot, title, detail): (Hsla, String, Option<String>) = match &self.printer {
            PrinterState::Searching => (theme.muted_foreground, tr!("Searching for printer", "Suche Drucker").to_string(), None),
            PrinterState::Missing { .. } => (theme.muted_foreground, tr!("No printer", "Kein Drucker").to_string(), None),
            PrinterState::Ready { model, media, .. } => {
                (theme.success, model.name.clone(), media.map(media_short))
            }
            PrinterState::Problem { name, message, .. } => (theme.danger, name.clone(), Some(message.clone())),
            PrinterState::Busy { product, .. } => (theme.warning, product.clone(), Some(tr!("busy", "belegt").to_string())),
        };
        let swatch = matches!(self.printer, PrinterState::Ready { .. }).then(|| {
            div()
                .w(px(26.))
                .h(px(16.))
                .rounded(px(3.))
                .border_1()
                .border_color(theme.border)
                .bg(rgb_of(look.tape))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb_of(look.ink))
                .child("Aa")
        });
        h_flex()
            .h(px(52.))
            .flex_none()
            .pl(if cfg!(target_os = "macos") { px(84.) } else { px(16.) }) // traffic lights
            .pr_4()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(
                div()
                    .font_family(theme::DISPLAY_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(17.))
                    .child("Labelwerk"),
            )
            .child(
                h_flex()
                    .id("printer-status")
                    .h(px(30.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(div().size(px(8.)).rounded_full().bg(dot))
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                    .children(swatch)
                    .children(detail.map(|d| {
                        div().text_xs().font_family(theme::MONO_FONT).text_color(theme.muted_foreground).child(d)
                    })),
            )
    }

    fn render_inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let continuous = self.media.kind == Kind::Continuous;
        let align = self.label.align;
        let loaded = self.printer.loaded_media().filter(|_| self.printer.model().is_some_and(|m| m.name == self.model.name));
        let unit = |u: &'static str| div().text_xs().font_family(theme::MONO_FONT).text_color(theme.muted_foreground).child(u);

        let tile = |id: &'static str, icon: IconName, title: &'static str, on: bool| {
            v_flex()
                .id(id)
                .flex_1()
                .h(px(64.))
                .gap_1()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .border_1()
                .cursor_pointer()
                .when(on, |d| d.border_color(theme.ring).bg(theme.selection))
                .when(!on, |d| d.border_color(theme.border).bg(theme.background).hover(|s| s.bg(theme.accent)))
                .child(Icon::new(icon).size(px(18.)).text_color(if on { theme.foreground } else { theme.muted_foreground }))
                .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(title))
        };

        let text = v_flex()
            .gap_2()
            .child(Self::eyebrow(tr!("Text", "Text"), cx))
            .child(Textarea::new(&self.text))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Select::new(&self.font).small().search_placeholder(tr!("Search fonts", "Schrift suchen")),
                        ),
                    )
                    .child(
                        ButtonGroup::new("style")
                            .outline()
                            .small()
                            .multiple(true)
                            .child(Button::new("bold").icon(IconName::Bold).selected(self.label.bold).tooltip(tr!("Bold", "Fett")))
                            .child(
                                Button::new("italic").icon(IconName::Italic).selected(self.label.italic).tooltip(tr!("Italic", "Kursiv")),
                            )
                            .on_click(cx.listener(|this, sel: &Vec<usize>, window, cx| {
                                this.label.bold = sel.contains(&0);
                                this.label.italic = sel.contains(&1);
                                this.changed(window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        ButtonGroup::new("align")
                            .outline()
                            .small()
                            .child(
                                Button::new("left")
                                    .icon(IconName::TextAlignStart)
                                    .selected(align == Align::Left)
                                    .tooltip(tr!("Align left", "Linksbündig")),
                            )
                            .child(
                                Button::new("center")
                                    .icon(IconName::TextAlignCenter)
                                    .selected(align == Align::Center)
                                    .tooltip(tr!("Center", "Zentriert")),
                            )
                            .child(
                                Button::new("right")
                                    .icon(IconName::TextAlignEnd)
                                    .selected(align == Align::Right)
                                    .tooltip(tr!("Align right", "Rechtsbündig")),
                            )
                            .on_click(cx.listener(|this, sel: &Vec<usize>, window, cx| {
                                this.label.align = match sel.first() {
                                    Some(0) => Align::Left,
                                    Some(2) => Align::Right,
                                    _ => Align::Center,
                                };
                                this.changed(window, cx);
                            })),
                    )
                    .child(div().flex_1().child(NumberInput::new(&self.size).small().suffix(unit("pt")))),
            );

        let design = v_flex()
            .gap_2()
            .child(Self::eyebrow(tr!("Design", "Gestaltung"), cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        tile("t-heading", IconName::Heading, tr!("Heading", "Überschrift"), self.label.heading).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.label.heading = !this.label.heading;
                                this.changed(window, cx);
                            }),
                        ),
                    )
                    .child(tile("t-qr", IconName::QrCode, tr!("QR code", "QR-Code"), self.label.qr).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.label.qr = !this.label.qr;
                            this.changed(window, cx);
                        },
                    )))
                    .child(
                        tile("t-frame", IconName::Square, tr!("Frame", "Rahmen"), self.label.frame).on_click(cx.listener(
                            |this, _, window, cx| {
                                this.label.frame = !this.label.frame;
                                this.changed(window, cx);
                            },
                        )),
                    ),
            )
            .when(self.label.qr, |s| s.child(Input::new(&self.qr_text).small().prefix(Icon::new(IconName::QrCode).xsmall())));

        let support_hint = match self.model.protocol.support {
            Support::Verified => None,
            Support::Documented => Some(tr!("Implemented per Brother's command reference", "Nach Brothers Befehlsreferenz umgesetzt")),
            Support::Assumed => Some(tr!("Untested, derived from related models", "Ungetestet, abgeleitet aus verwandten Modellen")),
            Support::Unsupported => Some(tr!("Not supported: different print protocol", "Nicht unterstützt: anderes Druckprotokoll")),
        };
        let tape = v_flex()
            .gap_2()
            .child(Self::eyebrow(tr!("Printer & Tape", "Drucker & Band"), cx))
            .child(Select::new(&self.model_select).small().search_placeholder(tr!("Search models", "Modell suchen")))
            .when_some(support_hint, |s, hint| s.child(div().text_xs().text_color(theme.muted_foreground).child(hint)))
            .child(Select::new(&self.media_select).small().menu_max_h(rems(28.)))
            .child(
                Switch::new("follow")
                    .checked(self.follow_printer)
                    .label(tr!("Use loaded tape", "Eingelegtes Band übernehmen"))
                    .small()
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        this.follow_printer = *checked;
                        if *checked {
                            this.use_loaded_media(window, cx);
                        }
                        store::save(&this.saved());
                        cx.notify();
                    })),
            )
            .when_some(loaded.filter(|l| l.id != self.media.id), |s, l| {
                s.child(
                    Button::new("use-loaded")
                        .small()
                        .outline()
                        .icon(IconName::RefreshCw)
                        .label(format!("{}: {}", tr!("Loaded", "Eingelegt"), media_name(l)))
                        .on_click(cx.listener(|this, _, window, cx| this.use_loaded_media(window, cx))),
                )
            })
            .when(continuous, |s| {
                s.child(Self::field(tr!("Length", "Länge"), NumberInput::new(&self.length).small().suffix(unit("mm")), cx))
            })
            .child(Self::field(tr!("Margin", "Rand"), NumberInput::new(&self.padding).small().suffix(unit("mm")), cx));

        v_flex()
            .id("inspector")
            .w(px(INSPECTOR_W))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .px_5()
            .py_5()
            .gap_6()
            .child(text)
            .child(design)
            .child(tape)
    }

    fn render_mat(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let surface = theme::canvas(theme.is_dark());
        let mat_ink = rgba(surface.ink);
        let Some(r) = &self.rendered else { return div().flex_1().bg(rgb(surface.bg)) };
        let g = r.geometry;
        let look = self.look();
        let view = window.viewport_size();
        let mat_w = (f32::from(view.width) - INSPECTOR_W).max(200.);
        let mat_h = (f32::from(view.height) - TITLE_H - BAR_H).max(200.);
        let (w_mm, h_mm) = g.label_mm();
        let s = mat_scale(view, w_mm, h_mm);
        let (lw, lh) = (w_mm * s, h_mm * s);
        let (left, top) = ((mat_w - lw) / 2., (mat_h - lh) / 2.);
        // the tape runs along the feed: horizontally when the text runs along it
        let horizontal = g.direction == Direction::Along;
        let tape_rgb = rgb_of(look.tape);

        let grid = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let step = 10. * s;
                let (minor, major) = if step >= 14. { (1, 5) } else { (5, 10) };
                let x0 = f32::from(bounds.origin.x) + (f32::from(bounds.size.width) - lw) / 2.;
                let y0 = f32::from(bounds.origin.y) + (f32::from(bounds.size.height) - lh) / 2.;
                let (bx, by) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let first = |origin: f32, start: f32| ((start - origin) / step).floor() as i32;
                for i in first(x0, bx)..=first(x0, bx + bw) + 1 {
                    if i % minor != 0 {
                        continue;
                    }
                    let x = x0 + i as f32 * step;
                    let color = if i % major == 0 { surface.grid_major } else { surface.grid };
                    window.paint_quad(fill(Bounds::new(point(px(x), px(by)), size(px(1.), px(bh))), rgba(color)));
                }
                for j in first(y0, by)..=first(y0, by + bh) + 1 {
                    if j % minor != 0 {
                        continue;
                    }
                    let y = y0 + j as f32 * step;
                    let color = if j % major == 0 { surface.grid_major } else { surface.grid };
                    window.paint_quad(fill(Bounds::new(point(px(bx), px(y)), size(px(bw), px(1.))), rgba(color)));
                }
            },
        )
        .absolute()
        .size_full();

        // Tape or liner beyond the label, fading out, with the cut marked.
        let ext = (if horizontal { lw } else { lh } * 0.45).clamp(40., 180.);
        let fade = |to_outside: bool, color: Hsla| {
            let (a, b) = (color.opacity(0.0), color.opacity(0.55));
            let angle = if horizontal { 90. } else { 180. };
            if to_outside {
                linear_gradient(angle, linear_color_stop(b, 0.), linear_color_stop(a, 1.))
            } else {
                linear_gradient(angle, linear_color_stop(a, 0.), linear_color_stop(b, 1.))
            }
        };
        let continuous = g.kind == Kind::Continuous;
        let strip_color = if continuous { tape_rgb } else { hsla(0., 0., 1., 0.5) };
        let gap = if continuous { 0. } else { 3. * s };
        let strip = |before: bool| {
            let d = div().absolute().bg(fade(!before, strip_color));
            if horizontal {
                d.left(px(if before { left - ext - gap } else { left + lw + gap })).top(px(top)).w(px(ext)).h(px(lh))
            } else {
                d.top(px(if before { top - ext - gap } else { top + lh + gap })).left(px(left)).w(px(lw)).h(px(ext))
            }
        };
        let cut = |at_start: bool| {
            let d = div().absolute().border_color(mat_ink.opacity(0.7));
            if horizontal {
                let x = if at_start { left } else { left + lw };
                d.left(px(x - 0.5)).top(px(top - 10.)).h(px(lh + 20.)).w(px(1.)).border_l_1().border_dashed()
            } else {
                let y = if at_start { top } else { top + lh };
                d.top(px(y - 0.5)).left(px(left - 10.)).w(px(lw + 20.)).h(px(1.)).border_t_1().border_dashed()
            }
        };

        // Measurement like on a drawing, along the feed only (the other side is the tape or label width, named
        // above), on the side the tape does not run through.
        let mono = |t: String| {
            div().font_family(theme::MONO_FONT).text_size(px(12.)).text_color(mat_ink).whitespace_nowrap().child(t)
        };
        let dim_y = top + lh + 26.;
        let dim_x = left + lw + 26.;
        let h_dim = div()
            .absolute()
            .left(px(left))
            .top(px(dim_y - 8.))
            .w(px(lw))
            .h(px(16.))
            .child(div().absolute().left_0().top(px(7.5)).w_full().h(px(1.)).bg(mat_ink.opacity(0.6)))
            .child(div().absolute().left_0().top_0().w(px(1.)).h_full().bg(mat_ink))
            .child(div().absolute().right_0().top_0().w(px(1.)).h_full().bg(mat_ink))
            .child(
                h_flex()
                    .absolute()
                    .size_full()
                    .justify_center()
                    .child(div().px_2().bg(rgb(surface.bg)).child(mono(format!("{} mm", mm(w_mm))))),
            );
        let v_dim = div()
            .absolute()
            .left(px(dim_x - 8.))
            .top(px(top))
            .w(px(16.))
            .h(px(lh))
            .child(div().absolute().top_0().left(px(7.5)).h_full().w(px(1.)).bg(mat_ink.opacity(0.6)))
            .child(div().absolute().top_0().left_0().h(px(1.)).w_full().bg(mat_ink))
            .child(div().absolute().bottom_0().left_0().h(px(1.)).w_full().bg(mat_ink))
            .child(
                div()
                    .absolute()
                    .left(px(22.))
                    .top(px(lh / 2. - 9.))
                    .child(mono(format!("{} mm", mm(h_mm)))),
            );

        let label = div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(lw))
            .h(px(lh))
            .shadow_lg()
            .border_1()
            .border_color(rgba(surface.edge))
            .when(g.kind == Kind::Round, |d| d.rounded_full())
            .when(g.kind == Kind::DieCut, |d| d.rounded(px(1.5 * s)))
            .when_some(self.preview.clone(), |d, p| d.child(img(p).size_full()));

        let mut info = vec![media_name(self.media).to_uppercase()];
        if matches!(self.printer, PrinterState::Ready { .. }) && self.model.family == Family::Pt {
            info.push(format!("{} / {}", look.tape_name, look.ink_name).to_uppercase());
        }
        let mut detail = Vec::new();
        if let Some(pt) = r.font_pt {
            let word = tr!("Font", "Schrift");
            detail.push(format!("{word} {} pt{}", mm(pt), if self.label.size_pt.is_none() { " · auto" } else { "" }));
        }
        if continuous && self.label.length_mm.is_none() {
            detail.push(tr!("Length follows the text", "Länge folgt dem Text").to_string());
        }
        let blocker = self.blocker().filter(|_| !self.printing);

        div()
            .flex_1()
            .min_w_0()
            .relative()
            .overflow_hidden()
            .bg(rgb(surface.bg))
            .child(grid)
            .child(strip(true))
            .child(strip(false))
            .when(continuous, |d| d.child(cut(true)).child(cut(false)))
            .child(label)
            .child(if horizontal { h_dim } else { v_dim })
            .child(
                v_flex()
                    .absolute()
                    .top(px(18.))
                    .left(px(20.))
                    .gap_1()
                    .child(div().font_family(theme::MONO_FONT).text_xs().text_color(mat_ink).child(info.join("  ·  ")))
                    .child(div().text_xs().text_color(mat_ink.opacity(0.7)).child(detail.join("  ·  "))),
            )
            .child(
                h_flex().absolute().top(px(12.)).right(px(14.)).gap_1().child(
                    Button::new("dots")
                        .small()
                        .ghost()
                        .selected(self.show_dots)
                        .icon(Icon::new(IconName::Grid3x3).text_color(mat_ink))
                        .child(div().text_color(mat_ink).child(tr!("Print dots", "Druckpunkte")))
                        .tooltip(tr!("Shows the individual dots the printer sets", "Die einzelnen Punkte zeigen, die der Drucker setzt"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_dots = !this.show_dots;
                            this.rerender(window, cx);
                        })),
                ).child(
                    Button::new("rotate")
                        .small()
                        .ghost()
                        .icon(Icon::new(IconName::RotateCw).text_color(mat_ink))
                        .child(div().text_color(mat_ink).child(tr!("Rotate", "Drehen")))
                        .tooltip(format!("{} ({})", tr!("Rotate text direction", "Textrichtung drehen"), shortcut_hint("R")))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_direction(window, cx))),
                ),
            )
            .child(
                v_flex()
                    .absolute()
                    .bottom(px(16.))
                    .left_0()
                    .right_0()
                    .items_center()
                    .gap_2()
                    .children(r.warnings.iter().map(|w| {
                        h_flex()
                            .gap_2()
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(theme.warning)
                            .text_color(theme.warning_foreground)
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(Icon::new(IconName::CircleAlert).xsmall())
                            .child(localize_warning(w))
                    }))
                    .children(blocker.map(|b| {
                        h_flex()
                            .gap_2()
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(hsla(0., 0., 0., 0.35))
                            .text_color(mat_ink)
                            .text_xs()
                            .child(Icon::new(IconName::Info).xsmall())
                            .child(b)
                    })),
            )
    }

    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let blocked = self.blocker().is_some();
        let label = if self.printing { tr!("Printing…", "Druckt …") } else { tr!("Print", "Drucken") };
        let thumbs = self.history.iter().zip(&self.thumbs).take(12).enumerate().map(|(i, (entry, thumb))| {
            let e = entry.clone();
            let (image, aspect) = thumb.clone();
            let h = 38.;
            div()
                .id(("history", i))
                .flex_none()
                .h(px(h))
                .w(px((h * aspect).clamp(24., 150.)))
                .rounded(px(3.))
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .cursor_pointer()
                .hover(|s| s.border_color(theme.ring))
                .child(img(image).size_full())
                .tooltip({
                    let t = entry.title();
                    move |w, cx| gpui_kit::component::tooltip::Tooltip::new(t.clone()).build(w, cx)
                })
                .on_click(cx.listener(move |this, _, window, cx| this.load_history(e.clone(), window, cx)))
        });
        h_flex()
            .h(px(BAR_H))
            .flex_none()
            .px_5()
            .gap_4()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                h_flex()
                    .id("history-strip")
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .overflow_x_scroll()
                    .when(!self.history.is_empty(), |d| d.child(div().pr_1().child(Self::eyebrow(tr!("Recently printed", "Zuletzt"), cx))))
                    .when(self.history.is_empty(), |d| {
                        d.child(div().text_sm().text_color(theme.muted_foreground).child(tr!(
                            "Printed labels appear here to reprint.",
                            "Gedruckte Etiketten erscheinen hier zum Nachdrucken."
                        )))
                    })
                    .children(thumbs),
            )
            .child(div().text_sm().text_color(theme.muted_foreground).child(tr!("Copies", "Anzahl")))
            .child(div().w(px(116.)).child(NumberInput::new(&self.copies_input)))
            .child(
                Button::new("print")
                    .primary()
                    .large()
                    .icon(IconName::Printer)
                    .label(label)
                    .loading(self.printing)
                    .disabled(blocked)
                    .tooltip(format!("{} ({})", tr!("Print", "Drucken"), shortcut_hint("P")))
                    .on_click(cx.listener(|this, _, window, cx| this.print(window, cx))),
            )
    }
}

/// Pixels per printer dot for the smooth preview: as many as the screen shows, so text edges stay smooth.
fn preview_k(g: &labelwerk_core::render::Geometry, window: &Window) -> u32 {
    let (w_mm, h_mm) = g.label_mm();
    let screen_px_per_mm = mat_scale(window.viewport_size(), w_mm, h_mm) * window.scale_factor();
    let max_k = ((6_000_000.0 / (g.label_w * g.label_h).max(1) as f32).sqrt() as u32).max(1);
    ((screen_px_per_mm * 25.4 / g.dpi as f32).ceil() as u32).clamp(1, 8).min(max_k)
}

/// Screen points per millimetre for a label on the mat of a window of this size.
fn mat_scale(view: Size<Pixels>, w_mm: f32, h_mm: f32) -> f32 {
    let mat_w = (f32::from(view.width) - INSPECTOR_W).max(200.);
    let mat_h = (f32::from(view.height) - TITLE_H - BAR_H).max(200.);
    ((mat_w - 260.) / w_mm).min((mat_h - 220.) / h_mm).clamp(0.5, MAX_PT_PER_MM)
}

/// The core's warnings are English; map them to German when the UI is German, else leave them as is.
fn localize_warning(w: &str) -> String {
    if !i18n::german() {
        return w.to_string();
    }
    match w {
        "Text does not fit at this size" => "Text passt in dieser Größe nicht aufs Etikett".into(),
        "QR code is too small for this label" => "QR-Code ist für dieses Etikett zu klein".into(),
        "Length adjusted to what the printer can do" => "Länge an die Grenzen des Druckers angepasst".into(),
        other => other.to_string(),
    }
}

/// Keyboard-shortcut hint for a tooltip: the Command symbol on macOS, "Ctrl"/"Strg" elsewhere.
fn shortcut_hint(key: &str) -> String {
    if cfg!(target_os = "macos") { format!("⌘{key}") } else { format!("{}+{key}", tr!("Ctrl", "Strg")) }
}

/// `a` moved towards `b` by `t`.
fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

fn rgb_of(c: [u8; 3]) -> Hsla {
    rgb(u32::from_be_bytes([0, c[0], c[1], c[2]])).into()
}

/// Short media name for the status pill: "24 mm", "62 × 29".
fn media_short(m: &Media) -> String {
    let (w, l) = m.nominal_mm();
    match m.kind {
        Kind::DieCut => format!("{} × {}", mm(w), mm(l)),
        Kind::Round => format!("Ø {}", mm(w)),
        Kind::Continuous => format!("{} mm", mm(w)),
    }
}

impl Focusable for LabelApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LabelApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let titlebar = self.render_titlebar(cx);
        let inspector = self.render_inspector(cx);
        let mat = self.render_mat(window, cx);
        let bar = self.render_bar(cx);
        v_flex()
            .key_context("Labelwerk")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &PrintLabel, window, cx| this.print(window, cx)))
            .on_action(cx.listener(|this, _: &RotateLabel, window, cx| this.toggle_direction(window, cx)))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme::UI_FONT)
            .child(titlebar)
            .child(h_flex().flex_1().min_h_0().items_stretch().child(inspector).child(mat))
            .child(bar)
    }
}
