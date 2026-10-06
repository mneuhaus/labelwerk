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
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IndexPath, Selectable as _, Sizable, StyledExt as _, WindowExt};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use labelwerk_core::media::dots_to_mm;
use labelwerk_core::render::preview_png;
use labelwerk_core::model::Support;
use labelwerk_core::{Align, Family, Kind, Label, Media, Model, PrintOptions, Rendered, Renderer, models};

use crate::printer::{self, PrinterState};
use crate::store::{self, HistoryEntry, Saved};

const SIDEBAR_W: f32 = 368.;
/// Never draw a label larger than this many screen points per millimetre (about 3x real size).
const MAX_PREVIEW_PT_PER_MM: f32 = 11.0;

gpui_kit::actions!(labelwerk, [PrintLabel, RotateLabel]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-p", PrintLabel, None),
        KeyBinding::new("ctrl-p", PrintLabel, None),
        KeyBinding::new("cmd-r", RotateLabel, None),
        KeyBinding::new("ctrl-r", RotateLabel, None),
    ]);
}

/// German name of a medium.
pub fn media_name(m: &Media) -> String {
    let (w, l) = m.nominal_mm();
    let (w, l) = (mm(w), mm(l));
    match (m.family, m.kind) {
        (Family::Pt, _) if m.is_tube() => format!("{w} mm Schrumpfschlauch"),
        (Family::Pt, _) => format!("{w} mm Band"),
        (_, Kind::Continuous) => format!("{w} mm Endlosband"),
        (_, Kind::DieCut) => format!("{w} × {l} mm"),
        (_, Kind::Round) => format!("Ø {w} mm rund"),
    }
}

fn mm(v: f32) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches(".0").replace('.', ",")
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
            .when(self.loaded, |d| d.child(div().text_xs().text_color(cx.theme().success).child("eingelegt")))
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
        Family::Pt => vec![group("Bänder", &|m| !m.is_tube()), group("Schrumpfschlauch", &|m| m.is_tube())],
        Family::Ql => vec![
            group("Endlosband", &|m| m.kind == Kind::Continuous),
            group("Etiketten", &|m| m.kind == Kind::DieCut),
            group("Rund", &|m| m.kind == Kind::Round),
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
                .placeholder("Text des Etiketts")
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
            InputState::new(window, cx).placeholder("Inhalt, leer = Etikettentext").default_value(label.qr_content.clone())
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
        self.preview = Some(Arc::new(Image::from_bytes(ImageFormat::Png, preview_png(&rendered))));
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
            self.printer = state;
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
            return Some("Druckt gerade …".into());
        }
        let empty = self.label.text.trim().is_empty() && !(self.label.qr && !self.label.qr_data().is_empty());
        if empty {
            return Some("Das Etikett ist leer".into());
        }
        match &self.printer {
            PrinterState::Searching => Some("Suche Drucker …".into()),
            PrinterState::Missing { .. } => Some(format!("{} per USB anschließen und einschalten", self.model.name)),
            PrinterState::Busy { queue: Some(_), .. } => None,
            PrinterState::Busy { message, .. } => Some(format!("Drucker belegt: {message}")),
            PrinterState::Problem { message, .. } => Some(message.clone()),
            PrinterState::Ready { model, .. } if model.name != self.model.name => {
                Some(format!("Angeschlossen ist ein {}", model.name))
            }
            PrinterState::Ready { media: Some(loaded), .. } if loaded.id != self.media.id => {
                Some(format!("Eingelegt ist {}", media_name(loaded)))
            }
            PrinterState::Ready { .. } => None,
        }
    }

    fn print(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reason) = self.blocker() {
            window.push_notification(Notification::warning(reason).title("Drucken nicht möglich"), cx);
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
                    Ok(()) => {
                        store::remember(&mut this.history, entry);
                        store::save(&this.saved());
                        let what = if copies == 1 { "1 Etikett".to_string() } else { format!("{copies} Etiketten") };
                        window.push_notification(Notification::success(format!("{what} auf {}", media_name(media))).title("Gedruckt"), cx);
                    }
                    Err(e) => {
                        window.push_notification(Notification::error(printer::german(&format!("{e:#}"))).title("Druck fehlgeschlagen"), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // MARK: rendering

    fn printer_badge(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (color, text, icon): (Hsla, String, IconName) = match &self.printer {
            PrinterState::Searching => (theme.muted_foreground, "Suche Drucker …".into(), IconName::LoaderCircle),
            PrinterState::Missing { .. } => (theme.muted_foreground, "Kein Drucker".into(), IconName::Unplug),
            PrinterState::Ready { model, media } => (
                theme.success,
                format!("{} · {}", model.name, media.map(media_name).unwrap_or_default()),
                IconName::Printer,
            ),
            PrinterState::Problem { name, message, .. } => (theme.danger, format!("{name} · {message}"), IconName::CircleAlert),
            PrinterState::Busy { product, .. } => (theme.warning, format!("{product} belegt"), IconName::CircleAlert),
        };
        h_flex()
            .id("printer-badge")
            .gap_2()
            .px_3()
            .py_1()
            .rounded_full()
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .child(Icon::new(icon).small().text_color(color))
            .child(div().text_color(theme.foreground).child(text))
    }

    fn section(title: &'static str, cx: &Context<Self>) -> Div {
        v_flex().gap_2().child(
            div().text_xs().font_semibold().text_color(cx.theme().muted_foreground).child(title.to_uppercase()),
        )
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let continuous = self.media.kind == Kind::Continuous;
        let align = self.label.align;
        let loaded = self.printer.loaded_media();

        let text = Self::section("Text", cx)
            .child(Textarea::new(&self.text))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Select::new(&self.font).small().search_placeholder("Schrift suchen")))
                    .child(
                        ButtonGroup::new("style")
                            .outline()
                            .small()
                            .multiple(true)
                            .child(Button::new("bold").icon(IconName::Bold).selected(self.label.bold).tooltip("Fett"))
                            .child(Button::new("italic").icon(IconName::Italic).selected(self.label.italic).tooltip("Kursiv"))
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
                            .child(Button::new("left").icon(IconName::TextAlignStart).selected(align == Align::Left).tooltip("Linksbündig"))
                            .child(Button::new("center").icon(IconName::TextAlignCenter).selected(align == Align::Center).tooltip("Zentriert"))
                            .child(Button::new("right").icon(IconName::TextAlignEnd).selected(align == Align::Right).tooltip("Rechtsbündig"))
                            .on_click(cx.listener(|this, sel: &Vec<usize>, window, cx| {
                                this.label.align = match sel.first() {
                                    Some(0) => Align::Left,
                                    Some(2) => Align::Right,
                                    _ => Align::Center,
                                };
                                this.changed(window, cx);
                            })),
                    )
                    .child(
                        div().flex_1().child(
                            NumberInput::new(&self.size)
                                .small()
                                .suffix(div().text_xs().text_color(theme.muted_foreground).child("pt")),
                        ),
                    ),
            );

        let support_hint = match self.model.protocol.support {
            Support::Verified => None,
            Support::Documented => Some("Nach Brothers Befehlsreferenz für dieses Modell umgesetzt"),
            Support::Assumed => Some("Für dieses Modell noch ungetestet (abgeleitet aus verwandten Modellen)"),
        };
        let media = Self::section("Drucker & Etikett", cx)
            .child(Select::new(&self.model_select).small().search_placeholder("Modell suchen"))
            .when_some(support_hint, |s, hint| s.child(div().text_xs().text_color(theme.muted_foreground).child(hint)))
            .child(Select::new(&self.media_select).small().menu_max_h(rems(28.)))
            .child(
                Switch::new("follow")
                    .checked(self.follow_printer)
                    .label("Eingelegte Rolle automatisch übernehmen")
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
            .when(continuous, |s| {
                s.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(64.)).text_sm().text_color(theme.muted_foreground).child("Länge"))
                        .child(
                            div().flex_1().child(
                                NumberInput::new(&self.length)
                                    .small()
                                    .suffix(div().text_xs().text_color(theme.muted_foreground).child("mm")),
                            ),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(64.)).text_sm().text_color(theme.muted_foreground).child("Rand"))
                    .child(
                        div().flex_1().child(
                            NumberInput::new(&self.padding)
                                .small()
                                .suffix(div().text_xs().text_color(theme.muted_foreground).child("mm")),
                        ),
                    ),
            )
            .when_some(loaded.filter(|l| l.id != self.media.id), |s, l| {
                s.child(
                    Button::new("use-loaded")
                        .small()
                        .outline()
                        .icon(IconName::RefreshCw)
                        .label(format!("Eingelegte Rolle nehmen: {}", media_name(l)))
                        .on_click(cx.listener(|this, _, window, cx| this.use_loaded_media(window, cx))),
                )
            });

        let extras = Self::section("Extras", cx)
            .child(
                Switch::new("qr")
                    .checked(self.label.qr)
                    .label("QR-Code")
                    .small()
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        this.label.qr = *checked;
                        this.changed(window, cx);
                    })),
            )
            .when(self.label.qr, |s| s.child(Input::new(&self.qr_text).small()))
            .child(
                Switch::new("frame")
                    .checked(self.label.frame)
                    .label("Rahmen")
                    .small()
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        this.label.frame = *checked;
                        this.changed(window, cx);
                    })),
            );

        let history = (!self.history.is_empty()).then(|| {
            Self::section("Zuletzt gedruckt", cx).child(v_flex().gap_1().children(self.history.iter().take(10).enumerate().map(
                |(i, entry)| {
                    let e = entry.clone();
                    let sub = Model::by_name(&entry.model)
                        .unwrap_or(self.model)
                        .media_by_key(&entry.media)
                        .map(media_name)
                        .unwrap_or_default();
                    h_flex()
                        .id(("history", i))
                        .px_2()
                        .py_1()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.accent))
                        .child(Icon::new(IconName::Tag).small().text_color(theme.muted_foreground))
                        .child(div().flex_1().min_w_0().truncate().text_sm().child(entry.title()))
                        .child(div().text_xs().text_color(theme.muted_foreground).child(sub))
                        .on_click(cx.listener(move |this, _, window, cx| this.load_history(e.clone(), window, cx)))
                },
            )))
        });

        v_flex()
            .id("sidebar")
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .p_4()
            .gap_5()
            .child(text)
            .child(media)
            .child(extras)
            .children(history)
    }

    fn render_preview(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let Some(r) = &self.rendered else { return div().flex_1() };
        let g = &r.geometry;
        let view = window.viewport_size();
        let avail_w = (f32::from(view.width) - SIDEBAR_W - 96.).max(100.);
        let avail_h = (f32::from(view.height) - 48. - 76. - 120.).max(100.);
        let (label_w_mm, label_h_mm) = g.label_mm();
        let pt_per_mm = (avail_w / label_w_mm).min(avail_h / label_h_mm).min(MAX_PREVIEW_PT_PER_MM);
        let mut info = vec![match self.media.kind {
            Kind::Continuous => format!("Länge {} mm", mm(dots_to_mm(g.along_dots(), g.dpi))),
            _ => format!("Druckbereich {} × {} mm", mm(dots_to_mm(g.print_w, g.dpi)), mm(dots_to_mm(g.print_h, g.dpi))),
        }];
        if let Some(pt) = r.font_pt {
            info.push(format!("Schrift {} pt{}", mm(pt), if self.label.size_pt.is_none() { " (auto)" } else { "" }));
        }
        if self.media.kind == Kind::Continuous && self.label.length_mm.is_none() {
            info.push("Länge passt sich an".into());
        }

        v_flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().font_semibold().child(media_name(self.media)))
                    .child(
                        Button::new("rotate")
                            .small()
                            .ghost()
                            .icon(IconName::RotateCw)
                            .label("Drehen")
                            .tooltip("Textrichtung drehen (⌘R)")
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_direction(window, cx))),
                    ),
            )
            .child(
                div()
                    .w(px(label_w_mm * pt_per_mm))
                    .h(px(label_h_mm * pt_per_mm))
                    .when_some(self.preview.clone(), |d, p| d.child(img(p).size_full())),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(info.join("  ·  ")))
            .children(r.warnings.iter().map(|w| {
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(Icon::new(IconName::CircleAlert).xsmall())
                    .child(match w.as_str() {
                        "Text does not fit at this size" => "Text passt in dieser Größe nicht aufs Etikett".to_string(),
                        "QR code is too small for this label" => "QR-Code ist für dieses Etikett zu klein".to_string(),
                        other => other.to_string(),
                    })
            }))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let blocker = self.blocker();
        let via_queue = matches!(self.printer, PrinterState::Busy { queue: Some(_), .. });
        let label = if self.printing { "Druckt …" } else { "Drucken" };
        h_flex()
            .h(px(76.))
            .px_6()
            .gap_4()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .when_some(blocker.clone().filter(|_| !self.printing), |d, b| d.child(b))
                    .when(blocker.is_none() && via_queue, |d| d.child("USB belegt, sende über die Druckwarteschlange")),
            )
            .child(div().text_sm().text_color(theme.muted_foreground).child("Anzahl"))
            .child(div().w(px(110.)).child(NumberInput::new(&self.copies_input)))
            .child(
                Button::new("print")
                    .primary()
                    .large()
                    .icon(IconName::Printer)
                    .label(label)
                    .loading(self.printing)
                    .disabled(blocker.is_some())
                    .tooltip("Drucken (⌘P)")
                    .on_click(cx.listener(|this, _, window, cx| this.print(window, cx))),
            )
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
        let sidebar = self.render_sidebar(cx);
        let preview = self.render_preview(window, cx);
        let footer = self.render_footer(cx);
        v_flex()
            .key_context("Labelwerk")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &PrintLabel, window, cx| this.print(window, cx)))
            .on_action(cx.listener(|this, _: &RotateLabel, window, cx| this.toggle_direction(window, cx)))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .h(px(48.))
                    .flex_none()
                    .pl(px(84.)) // room for the macOS traffic lights
                    .pr_4()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().font_semibold().child("Labelwerk"))
                    .child(self.printer_badge(cx)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(sidebar)
                    .child(v_flex().flex_1().min_w_0().h_full().bg(theme.muted).child(preview).child(footer)),
            )
    }
}
