//! Label document and the renderer that turns it into a printer page.
//!
//! The label is designed in reading orientation ("design space"). For text that runs along the tape the
//! design is rotated a quarter turn into printer orientation, where rows are raster lines.

use cosmic_text::{Align as CtAlign, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight};
use qrcode::{EcLevel, QrCode};
use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::media::{Kind, Media, dots_to_mm, mm_to_dots};
use crate::model::Model;

/// Line height as a multiple of the font size.
pub const LINE_SPACING: f32 = 1.15;
/// Longest continuous label the app offers (the printer takes up to 3 m).
pub const MAX_LENGTH_MM: f32 = 1000.0;
/// Frame line width in mm.
const FRAME_MM: f32 = 0.42;
const COVERAGE_THRESHOLD: u8 = 128;
/// Label font shipped with Labelwerk (SIL Open Font License), so labels look the same on every computer.
pub const BUNDLED_FONT: &str = "Barlow Semi Condensed";
const BUNDLED_FONT_FILES: &[&[u8]] = &[
    include_bytes!("../fonts/BarlowSemiCondensed-Regular.ttf"),
    include_bytes!("../fonts/BarlowSemiCondensed-Italic.ttf"),
    include_bytes!("../fonts/BarlowSemiCondensed-Bold.ttf"),
    include_bytes!("../fonts/BarlowSemiCondensed-BoldItalic.ttf"),
];
/// Families tried in order when a label names none or one that is not installed.
const PREFERRED_FONTS: &[&str] = &[BUNDLED_FONT, "Helvetica Neue", "Helvetica", "Arial", "Segoe UI", "DejaVu Sans"];
/// Size of an emphasised first line relative to the other lines.
pub const HEADING_SCALE: f32 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

/// Which way the text runs relative to the tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Along the feed: the label grows in length (typical for endless tape).
    Along,
    /// Across the print head: lines run over the tape's width.
    Across,
}

impl Direction {
    /// The natural direction for a medium: along the longer side, across for round labels.
    pub fn default_for(media: &Media) -> Direction {
        match media.kind {
            Kind::Continuous => Direction::Along,
            Kind::DieCut if media.print_length > media.print_width => Direction::Along,
            _ => Direction::Across,
        }
    }

    pub fn flipped(self) -> Direction {
        match self {
            Direction::Along => Direction::Across,
            Direction::Across => Direction::Along,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Label {
    pub text: String,
    /// Font family; empty or unknown falls back to a common sans serif.
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    /// Fixed size in points, `None` = as large as fits.
    pub size_pt: Option<f32>,
    pub align: Align,
    /// `None` = the medium's natural direction.
    pub direction: Option<Direction>,
    /// Total length of a continuous label in mm, `None` = as long as the content needs.
    pub length_mm: Option<f32>,
    /// Blank space between the printable edge and the content.
    pub padding_mm: f32,
    pub qr: bool,
    /// QR code content; empty encodes the label text.
    pub qr_content: String,
    pub frame: bool,
    /// First line larger and bold, the rest as normal text.
    pub heading: bool,
}

impl Default for Label {
    fn default() -> Self {
        Self {
            text: String::new(),
            font: String::new(),
            bold: false,
            italic: false,
            size_pt: None,
            align: Align::Center,
            direction: None,
            length_mm: None,
            padding_mm: 1.0,
            qr: false,
            qr_content: String::new(),
            frame: false,
            heading: false,
        }
    }
}

impl Label {
    pub fn direction_for(&self, media: &Media) -> Direction {
        self.direction.unwrap_or_else(|| Direction::default_for(media))
    }

    pub fn qr_data(&self) -> &str {
        if self.qr_content.trim().is_empty() { self.text.trim() } else { self.qr_content.trim() }
    }
}

/// Where the printable area sits on the physical label, in design orientation and dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub kind: Kind,
    pub dpi: u32,
    pub direction: Direction,
    pub label_w: u32,
    pub label_h: u32,
    pub print_x: u32,
    pub print_y: u32,
    pub print_w: u32,
    pub print_h: u32,
}

impl Geometry {
    /// `feed`: blank tape before and after a continuous label, in dots.
    fn new(media: &Media, direction: Direction, lines: u32, feed: u32) -> Self {
        let dpi = media.dpi;
        let across = mm_to_dots(media.width_mm(), dpi).max(media.print_width);
        let (along, along_offset) = match media.kind {
            Kind::Continuous => (lines + 2 * feed, feed),
            _ => {
                let length = mm_to_dots(media.length_mm(), dpi);
                (length, length.saturating_sub(lines) / 2)
            }
        };
        let across_offset = across.saturating_sub(media.print_width) / 2;
        match direction {
            Direction::Along => Self {
                kind: media.kind,
                dpi,
                direction,
                label_w: along,
                label_h: across,
                print_x: along_offset,
                print_y: across_offset,
                print_w: lines,
                print_h: media.print_width,
            },
            Direction::Across => Self {
                kind: media.kind,
                dpi,
                direction,
                label_w: across,
                label_h: along,
                print_x: across_offset,
                print_y: along_offset,
                print_w: media.print_width,
                print_h: lines,
            },
        }
    }

    /// Length of the label along the feed, in dots (the variable side of continuous tape).
    pub fn along_dots(&self) -> u32 {
        match self.direction {
            Direction::Along => self.label_w,
            Direction::Across => self.label_h,
        }
    }

    pub fn label_mm(&self) -> (f32, f32) {
        (dots_to_mm(self.label_w, self.dpi), dots_to_mm(self.label_h, self.dpi))
    }
}

#[derive(Debug, Clone)]
pub struct Rendered {
    /// Printable area in reading orientation.
    pub design: Bitmap,
    /// The same in printer orientation, ready for `encode_job`.
    pub page: Bitmap,
    pub geometry: Geometry,
    /// Font size actually used, in points (interesting when it was chosen automatically).
    pub font_pt: Option<f32>,
    /// Problems the user should see, e.g. text that does not fit.
    pub warnings: Vec<String>,
    plan: Plan,
}

/// Everything placed on the printable area, in printer dots, ready to be drawn at any integer scale: once
/// at scale 1 for the printer, and larger for a smooth on-screen preview with the very same layout.
#[derive(Debug, Clone)]
struct Plan {
    w: u32,
    h: u32,
    round: bool,
    text: Option<TextPlan>,
    qr: Option<QrPlan>,
    /// (padding, line width) of the frame
    frame: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
struct TextPlan {
    text: String,
    family: String,
    bold: bool,
    italic: bool,
    heading: bool,
    align: Align,
    px: f32,
    x: f32,
    y: f32,
    block_w: f32,
}

#[derive(Debug, Clone)]
struct QrPlan {
    x: u32,
    y: u32,
    module: u32,
    n: u32,
    dark: Vec<bool>,
}

/// Font attributes plus the label's line structure.
struct TextStyle<'a> {
    attrs: Attrs<'a>,
    heading: bool,
}

pub struct Renderer {
    fonts: FontSystem,
    cache: SwashCache,
    families: Vec<String>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// Loads the system fonts; takes a moment, create it once.
    pub fn new() -> Self {
        let mut fonts = FontSystem::new();
        for font in BUNDLED_FONT_FILES {
            fonts.db_mut().load_font_data(font.to_vec());
        }
        let mut families: Vec<String> = fonts
            .db()
            .faces()
            .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
            .filter(|n| !n.starts_with('.') && !n.starts_with('#'))
            .collect();
        families.sort_by_key(|f| f.to_lowercase());
        families.dedup();
        Self { fonts, cache: SwashCache::new(), families }
    }

    /// Installed font families, sorted.
    pub fn families(&self) -> &[String] {
        &self.families
    }

    pub fn default_family(&self) -> String {
        PREFERRED_FONTS
            .iter()
            .find(|f| self.families.iter().any(|x| x == *f))
            .map(|f| f.to_string())
            .or_else(|| self.families.first().cloned())
            .unwrap_or_default()
    }

    fn family_for(&self, label: &Label) -> String {
        if !label.font.is_empty() && self.families.iter().any(|f| f == &label.font) {
            label.font.clone()
        } else {
            self.default_family()
        }
    }

    pub fn render(&mut self, label: &Label, model: &Model, media: &Media) -> Rendered {
        let dpi = model.dpi;
        let mm = |v: f32| mm_to_dots(v, dpi);
        let feed = crate::protocol::margin_dots(model, media, &crate::protocol::PrintOptions::default()) as u32;
        let (min_lines, max_lines) = model.length_limits();
        let frame_dots = mm(FRAME_MM).max(2);
        let direction = label.direction_for(media);
        let family = self.family_for(label);
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(if label.bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if label.italic { Style::Italic } else { Style::Normal });
        let text = label.text.trim_end_matches(['\n', ' ']);
        let style = TextStyle { attrs, heading: label.heading && text.contains('\n') };
        let mut warnings = Vec::new();

        let across = media.print_width;
        let pad = mm(label.padding_mm.max(0.0));
        let inset = pad + if label.frame { frame_dots + pad.max(mm(2.0)) } else { 0 };
        let gap = mm(1.5).max(pad);
        let qr = if label.qr && !label.qr_data().is_empty() {
            match QrCode::with_error_correction_level(label.qr_data().as_bytes(), EcLevel::M) {
                Ok(code) => Some(code),
                Err(e) => {
                    warnings.push(format!("QR code: {e}"));
                    None
                }
            }
        } else {
            None
        };
        let fixed_px = label.size_pt.map(|pt| pt * dpi as f32 / 72.0);
        let measure_100 = if text.is_empty() { (0.0, 0.0) } else { self.measure(text, &style, 100.0) };

        // Length along the feed for continuous tape when it follows the content.
        let auto_lines = |this: &mut Self| -> u32 {
            if text.is_empty() && qr.is_none() {
                return min_lines;
            }
            let content = match direction {
                Direction::Along => {
                    let box_h = across.saturating_sub(2 * inset) as f32;
                    let qr_side = if qr.is_some() { box_h } else { 0.0 };
                    let text_w = if text.is_empty() {
                        0.0
                    } else {
                        let px = fixed_px.unwrap_or(100.0 * box_h / measure_100.1.max(1.0));
                        this.measure(text, &style, px).0
                    };
                    qr_side + if qr.is_some() && text_w > 0.0 { gap as f32 } else { 0.0 } + text_w
                }
                Direction::Across => {
                    let box_w = across.saturating_sub(2 * inset) as f32;
                    let qr_side = if qr.is_some() { if text.is_empty() { box_w } else { box_w * 0.4 } } else { 0.0 };
                    let text_box_w = box_w - if qr.is_some() && !text.is_empty() { qr_side + gap as f32 } else { qr_side };
                    let text_h = if text.is_empty() {
                        0.0
                    } else {
                        let px = fixed_px.unwrap_or(100.0 * text_box_w / measure_100.0.max(1.0));
                        this.measure(text, &style, px).1
                    };
                    text_h.max(qr_side)
                }
            };
            content.ceil() as u32 + 2 * inset
        };

        let lines = match media.kind {
            Kind::Continuous => {
                let lines = match label.length_mm {
                    Some(len) => mm(len.min(MAX_LENGTH_MM)).saturating_sub(2 * feed),
                    None => auto_lines(self),
                };
                let clamped = lines.clamp(min_lines, max_lines);
                if label.length_mm.is_some() && clamped != lines {
                    warnings.push("Length adjusted to what the printer can do".into());
                }
                clamped
            }
            _ => media.print_length,
        };
        let geometry = Geometry::new(media, direction, lines, feed);
        let (w, h) = (geometry.print_w, geometry.print_h);
        let mut plan = Plan { w, h, round: media.kind == Kind::Round, text: None, qr: None, frame: None };

        // Content box; round labels use the square inside the circle.
        let (mut bx, mut by, mut bw, mut bh) = (inset, inset, w.saturating_sub(2 * inset), h.saturating_sub(2 * inset));
        if media.kind == Kind::Round {
            let side = (w.min(h) as f32 / std::f32::consts::SQRT_2) as u32;
            let side = side.saturating_sub(2 * inset.saturating_sub(pad));
            bx = (w - side) / 2;
            by = (h - side) / 2;
            bw = side;
            bh = side;
        }

        if let Some(code) = &qr {
            let side = if text.is_empty() { bw.min(bh) } else { bh.min((bw as f32 * 0.45) as u32) };
            let n = code.width() as u32;
            let module = side / n;
            if module == 0 {
                warnings.push("QR code is too small for this label".into());
            } else {
                let size = module * n;
                let qx = if text.is_empty() { bx + (bw - size) / 2 } else { bx };
                let qy = by + (bh - size) / 2;
                let dark = code.to_colors().iter().map(|c| *c == qrcode::Color::Dark).collect();
                plan.qr = Some(QrPlan { x: qx, y: qy, module, n, dark });
                if !text.is_empty() {
                    let used = size + gap;
                    bx += used;
                    bw = bw.saturating_sub(used);
                }
            }
        }

        let mut font_pt = None;
        if !text.is_empty() && bw > 0 && bh > 0 {
            let (bwf, bhf) = (bw as f32, bh as f32);
            let mut px = match fixed_px {
                Some(px) => px,
                None => {
                    let continuous_along = media.kind == Kind::Continuous && label.length_mm.is_none();
                    let by_height = 100.0 * bhf / measure_100.1.max(1.0);
                    let by_width = 100.0 * bwf / measure_100.0.max(1.0);
                    match (continuous_along, direction) {
                        (true, Direction::Along) => by_height,
                        (true, Direction::Across) => by_width,
                        _ => by_height.min(by_width),
                    }
                }
            };
            let mut size = self.measure(text, &style, px);
            if fixed_px.is_none() {
                for _ in 0..12 {
                    if size.0 <= bwf + 0.5 && size.1 <= bhf + 0.5 {
                        break;
                    }
                    px *= 0.97;
                    size = self.measure(text, &style, px);
                }
            } else if size.0 > bwf + 0.5 || size.1 > bhf + 0.5 {
                warnings.push("Text does not fit at this size".into());
            }
            font_pt = Some(px * 72.0 / dpi as f32);
            let tx = match label.align {
                Align::Left => bx as f32,
                Align::Center => bx as f32 + (bwf - size.0) / 2.0,
                Align::Right => bx as f32 + bwf - size.0,
            };
            let ty = by as f32 + (bhf - size.1) / 2.0;
            plan.text = Some(TextPlan {
                text: text.to_string(),
                family: family.clone(),
                bold: label.bold,
                italic: label.italic,
                heading: style.heading,
                align: label.align,
                px,
                x: tx,
                y: ty,
                block_w: size.0,
            });
        }
        if label.frame {
            plan.frame = Some((pad, frame_dots));
        }

        let cov = self.draw(&plan, 1);
        let design = Bitmap::from_coverage(w, h, &cov, COVERAGE_THRESHOLD);
        let page = match direction {
            Direction::Along => design.rotate_cw(),
            Direction::Across => design.clone(),
        };
        Rendered { design, page, geometry, font_pt, warnings, plan }
    }

    /// Antialiased coverage of the printable area at `k` pixels per printer dot.
    fn draw(&mut self, plan: &Plan, k: u32) -> Vec<u8> {
        let (w, h) = (plan.w * k, plan.h * k);
        let mut cov = vec![0u8; (w * h) as usize];
        if let Some(q) = &plan.qr {
            for (i, &dark) in q.dark.iter().enumerate() {
                if dark {
                    let (mx, my) = (i as u32 % q.n, i as u32 / q.n);
                    let m = q.module * k;
                    fill(&mut cov, w, (q.x + mx * q.module) * k, (q.y + my * q.module) * k, m, m);
                }
            }
        }
        if let Some(t) = &plan.text {
            let attrs = Attrs::new()
                .family(Family::Name(&t.family))
                .weight(if t.bold { Weight::BOLD } else { Weight::NORMAL })
                .style(if t.italic { Style::Italic } else { Style::Normal });
            let style = TextStyle { attrs, heading: t.heading };
            let kf = k as f32;
            let (ox, oy) = ((t.x * kf).round() as i32, (t.y * kf).round() as i32);
            self.draw_text(&t.text, &style, t.px * kf, t.align, t.block_w * kf, &mut cov, w, h, ox, oy);
        }
        if let Some((pad, line)) = plan.frame {
            if plan.round {
                ring(&mut cov, w, h, line * k);
            } else {
                let (p, f) = (pad * k, line * k);
                let (fw, fh) = (w.saturating_sub(2 * p), h.saturating_sub(2 * p));
                fill(&mut cov, w, p, p, fw, f);
                fill(&mut cov, w, p, (p + fh).saturating_sub(f), fw, f);
                fill(&mut cov, w, p, p, f, fh);
                fill(&mut cov, w, (p + fw).saturating_sub(f), p, f, fh);
            }
        }
        if plan.round {
            mask_circle(&mut cov, w, h);
        }
        cov
    }

    /// The whole label as RGBA for the screen: same layout as the print, but drawn antialiased at `k` pixels
    /// per printer dot instead of the printer's black-and-white dots.
    pub fn preview_smooth(&mut self, r: &Rendered, k: u32, style: &PreviewStyle) -> (u32, u32, Vec<u8>) {
        let k = k.max(1);
        let cov = self.draw(&r.plan, k);
        let pw = r.plan.w * k;
        compose(&r.geometry, k, style, |x, y| cov[(y * pw + x) as usize])
    }

    fn buffer(&mut self, text: &str, style: &TextStyle, px: f32, align: Align, width: Option<f32>) -> Buffer {
        let mut buf = Buffer::new(&mut self.fonts, Metrics::new(px, px * LINE_SPACING));
        buf.set_size(width, None);
        let align = match align {
            Align::Left => CtAlign::Left,
            Align::Center => CtAlign::Center,
            Align::Right => CtAlign::Right,
        };
        match (style.heading, text.split_once('\n')) {
            (true, Some((first, rest))) => {
                let big = px * HEADING_SCALE;
                let head = style.attrs.clone().weight(Weight::BOLD).metrics(Metrics::new(big, big * LINE_SPACING));
                let first = &text[..first.len() + 1]; // keep the line break with the heading
                let spans = [(first, head), (rest, style.attrs.clone())];
                buf.set_rich_text(spans, &style.attrs, Shaping::Advanced, Some(align));
            }
            _ => buf.set_text(text, &style.attrs, Shaping::Advanced, Some(align)),
        }
        buf.shape_until_scroll(&mut self.fonts, false);
        buf
    }

    /// Width and height of the laid-out text block at `px`.
    fn measure(&mut self, text: &str, style: &TextStyle, px: f32) -> (f32, f32) {
        let buf = self.buffer(text, style, px, Align::Left, None);
        buf.layout_runs().fold((0.0f32, 0.0f32), |(w, h), run| (w.max(run.line_w), h.max(run.line_top + run.line_height)))
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        text: &str,
        style: &TextStyle,
        px: f32,
        align: Align,
        block_w: f32,
        cov: &mut [u8],
        w: u32,
        h: u32,
        ox: i32,
        oy: i32,
    ) {
        let mut buf = self.buffer(text, style, px, align, Some(block_w.ceil() + 1.0));
        buf.draw(&mut self.fonts, &mut self.cache, Color::rgb(0, 0, 0), |x, y, rw, rh, color| {
            let a = color.a();
            if a == 0 {
                return;
            }
            for yy in y..y + rh as i32 {
                for xx in x..x + rw as i32 {
                    let (px, py) = (xx + ox, yy + oy);
                    if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                        let i = (py as u32 * w + px as u32) as usize;
                        cov[i] = cov[i].max(a);
                    }
                }
            }
        });
    }
}

/// Colours of the preview: the tape or paper, the print, and the printable-area outline (`None` = hidden).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewStyle {
    pub paper: [u8; 3],
    pub ink: [u8; 3],
    pub outline: Option<[u8; 3]>,
}

impl Default for PreviewStyle {
    fn default() -> Self {
        Self { paper: [255, 255, 255], ink: [17, 17, 17], outline: Some([205, 212, 220]) }
    }
}

/// The whole label as RGBA at printer resolution: paper in its real shape (transparent around it), the
/// printed pixels and optionally a dashed outline of the printable area.
pub fn preview_rgba(r: &Rendered, style: &PreviewStyle) -> (u32, u32, Vec<u8>) {
    compose(&r.geometry, 1, style, |x, y| if r.design.get(x, y) { 255 } else { 0 })
}

/// Paint the label shape at `k` pixels per dot and blend the print in, `ink(x, y)` being the print's
/// coverage at a pixel of the (scaled) printable area.
fn compose(g: &Geometry, k: u32, style: &PreviewStyle, ink: impl Fn(u32, u32) -> u8) -> (u32, u32, Vec<u8>) {
    let (w, h) = (g.label_w * k, g.label_h * k);
    let (print_x, print_y, print_w, print_h) = (g.print_x * k, g.print_y * k, g.print_w * k, g.print_h * k);
    let mut px = vec![0u8; (w * h * 4) as usize];
    let corner = match g.kind {
        Kind::DieCut => (mm_to_dots(1.5, g.dpi) * k) as f32,
        _ => 0.0,
    };
    let inside = |x: u32, y: u32| -> bool {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        match g.kind {
            Kind::Round => {
                let r = w.min(h) as f32 / 2.0;
                (fx - w as f32 / 2.0).powi(2) + (fy - h as f32 / 2.0).powi(2) <= r * r
            }
            Kind::DieCut => {
                let cx = fx.clamp(corner, w as f32 - corner);
                let cy = fy.clamp(corner, h as f32 - corner);
                (fx - cx).powi(2) + (fy - cy).powi(2) <= corner * corner
            }
            Kind::Continuous => true,
        }
    };
    let dash = |i: u32| (i / (12 * k)).is_multiple_of(2);
    let line = k as i64;
    for y in 0..h {
        for x in 0..w {
            if !inside(x, y) {
                continue;
            }
            let (dx, dy) = (x as i64 - print_x as i64, y as i64 - print_y as i64);
            let (pw, ph) = (print_w as i64, print_h as i64);
            let in_print = dx >= 0 && dy >= 0 && dx < pw && dy < ph;
            let on_outline = {
                let near = |a: i64, edge: i64| (a - edge).abs() <= line;
                let vertical = (near(dx, -line) || near(dx, pw)) && dy >= -line && dy <= ph && dash(y);
                let horizontal = (near(dy, -line) || near(dy, ph)) && dx >= -line && dx <= pw && dash(x);
                g.kind != Kind::Round && (vertical || horizontal)
            };
            let base = match on_outline.then_some(style.outline).flatten() {
                Some(outline) => outline,
                None => style.paper,
            };
            let a = if in_print { ink(dx as u32, dy as u32) as u32 } else { 0 };
            let mix = |p: u8, i: u8| ((p as u32 * (255 - a) + i as u32 * a) / 255) as u8;
            let i = ((y * w + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[mix(base[0], style.ink[0]), mix(base[1], style.ink[1]), mix(base[2], style.ink[2]), 255]);
        }
    }
    (w, h, px)
}

pub fn preview_png(r: &Rendered, style: &PreviewStyle) -> Vec<u8> {
    let (w, h, rgba) = preview_rgba(r, style);
    crate::bitmap::encode_png_rgba(w, h, &rgba)
}

fn fill(cov: &mut [u8], w: u32, x: u32, y: u32, fw: u32, fh: u32) {
    let h = cov.len() as u32 / w;
    for yy in y.min(h)..(y + fh).min(h) {
        for xx in x.min(w)..(x + fw).min(w) {
            cov[(yy * w + xx) as usize] = 255;
        }
    }
}

fn ring(cov: &mut [u8], w: u32, h: u32, thickness: u32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let r_out = w.min(h) as f32 / 2.0 - 1.0;
    let r_in = r_out - thickness as f32;
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            if d <= r_out && d >= r_in {
                cov[(y * w + x) as usize] = 255;
            }
        }
    }
}

/// Clear everything outside the circle inscribed in the printable square.
fn mask_circle(cov: &mut [u8], w: u32, h: u32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let r = w.min(h) as f32 / 2.0;
    for y in 0..h {
        for x in 0..w {
            if (x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2) > r * r {
                cov[(y * w + x) as usize] = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ql() -> &'static Model {
        Model::by_name("QL-1100").unwrap()
    }

    fn pt() -> &'static Model {
        Model::by_name("PT-P710BT").unwrap()
    }

    #[test]
    fn continuous_label_grows_with_text() {
        let mut r = Renderer::new();
        let media = ql().media_by_key("62").unwrap();
        let short = r.render(&Label { text: "M3".into(), ..Label::default() }, ql(), media);
        let long = r.render(&Label { text: "M3 Schrauben 10 mm".into(), ..Label::default() }, ql(), media);
        assert_eq!(short.page.width, media.print_width);
        assert!(long.page.height > short.page.height);
        assert!(long.page.black_pixels() > 0);
        assert_eq!(long.geometry.direction, Direction::Along);
        assert!(long.warnings.is_empty(), "{:?}", long.warnings);
    }

    #[test]
    fn auto_size_fills_the_tape_height() {
        let mut r = Renderer::new();
        for (model, key) in [(ql(), "62"), (pt(), "24"), (pt(), "12")] {
            let media = model.media_by_key(key).unwrap();
            let out = r.render(&Label { text: "Hg".into(), padding_mm: 0.0, ..Label::default() }, model, media);
            // design: along the tape horizontally, across the tape vertically
            let (_, y0, _, y1) = out.design.ink_bounds().unwrap();
            let ink = (y1 - y0) as f32 / out.design.height as f32;
            assert!(ink > 0.6, "{} {key}: text uses only {:.0}% of the tape width", model.name, ink * 100.0);
        }
    }

    #[test]
    fn die_cut_page_has_the_media_size() {
        let mut r = Renderer::new();
        for key in ["62x29", "29x90", "d24", "102x152"] {
            let media = ql().media_by_key(key).unwrap();
            let label = Label { text: "Werkstatt\nRegal 3".into(), qr: true, frame: true, ..Label::default() };
            let out = r.render(&label, ql(), media);
            assert_eq!((out.page.width, out.page.height), (media.print_width, media.print_length), "{key}");
            assert!(out.page.black_pixels() > 0, "{key}");
            assert!(out.warnings.is_empty(), "{key}: {:?}", out.warnings);
        }
    }

    #[test]
    fn fixed_length_and_size() {
        let mut r = Renderer::new();
        let media = ql().media_by_key("29").unwrap();
        let label = Label { text: "Kabel".into(), length_mm: Some(50.0), size_pt: Some(24.0), ..Label::default() };
        let out = r.render(&label, ql(), media);
        assert_eq!(out.page.height, mm_to_dots(50.0, 300) - 2 * 35);
        assert!((out.font_pt.unwrap() - 24.0).abs() < 0.01);
    }

    #[test]
    fn pt_tape_renders_at_180_dpi() {
        let mut r = Renderer::new();
        let media = pt().media_by_key("24").unwrap();
        let label = Label { text: "Kabel".into(), length_mm: Some(50.0), ..Label::default() };
        let out = r.render(&label, pt(), media);
        assert_eq!(out.page.width, 128);
        assert_eq!(out.page.height, mm_to_dots(50.0, 180) - 2 * 14);
        let (w, h) = out.geometry.label_mm();
        assert!((w - 50.0).abs() < 0.3 && (h - 24.0).abs() < 0.3, "{w} x {h}");
    }

    #[test]
    fn bundled_font_is_the_default_and_heading_is_bigger() {
        let mut r = Renderer::new();
        assert_eq!(r.default_family(), BUNDLED_FONT);
        let media = ql().media_by_key("62x29").unwrap();
        let text = "Werkstatt\nRegal 3".to_string();
        let plain = r.render(&Label { text: text.clone(), size_pt: Some(20.0), ..Label::default() }, ql(), media);
        let head = r.render(&Label { text, size_pt: Some(20.0), heading: true, ..Label::default() }, ql(), media);
        let height = |b: &Bitmap| b.ink_bounds().map(|(_, y0, _, y1)| y1 - y0).unwrap();
        assert!(height(&head.design) > height(&plain.design), "the heading line makes the block taller");
    }

    #[test]
    fn smooth_preview_has_the_print_layout() {
        let mut r = Renderer::new();
        let media = pt().media_by_key("24").unwrap();
        let out = r.render(&Label { text: "Hallo".into(), ..Label::default() }, pt(), media);
        let (w, h, rgba) = r.preview_smooth(&out, 4, &PreviewStyle::default());
        assert_eq!((w, h), (out.geometry.label_w * 4, out.geometry.label_h * 4));
        // where the print has ink, the smooth preview is dark too (same layout, finer edges)
        let g = out.geometry;
        let dark = |x: u32, y: u32| rgba[((y * w + x) * 4) as usize] < 128;
        let print_dark = (0..out.design.height).flat_map(|y| (0..out.design.width).map(move |x| (x, y)))
            .filter(|&(x, y)| out.design.get(x, y))
            .filter(|&(x, y)| dark((g.print_x + x) * 4 + 2, (g.print_y + y) * 4 + 2))
            .count();
        assert!(print_dark as f32 > 0.85 * out.design.black_pixels() as f32, "layouts differ");
    }

    #[test]
    fn oversized_fixed_text_warns() {
        let mut r = Renderer::new();
        let media = ql().media_by_key("62x29").unwrap();
        let out = r.render(&Label { text: "Viel zu groß".into(), size_pt: Some(200.0), ..Label::default() }, ql(), media);
        assert!(!out.warnings.is_empty());
    }

    #[test]
    fn along_text_reads_left_to_right_from_the_leading_edge() {
        let mut r = Renderer::new();
        let media = ql().media_by_key("62").unwrap();
        let label = Label { text: "I".into(), qr: true, qr_content: "x".into(), align: Align::Left, padding_mm: 0.0, ..Label::default() };
        let out = r.render(&label, ql(), media);
        // The QR code sits left of the text in design space, so it must leave the printer first.
        let first_ink_row = (0..out.page.height).find(|&y| out.page.row(y).iter().any(|&p| p != 0)).unwrap();
        assert!(first_ink_row < 5, "QR should start at the leading edge, first ink at row {first_ink_row}");
    }
}
