//! `labelwerk`: print labels on Brother QL and PT (P-touch) printers from the command line.
//!
//! Exit codes: 0 ok, 1 error, 2 printer not ready (no printer, wrong media, cover open, ...).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use labelwerk_core::protocol::{decode_job, lines_to_bitmap};
use labelwerk_core::render::preview_png;
use labelwerk_core::transport::{self, PrintEvent, Probe, UsbPrinter};
use labelwerk_core::{Align, Direction, Label, Media, Model, PrintOptions, Renderer, encode_job, models};

#[derive(Parser)]
#[command(name = "labelwerk", version, about = "Labels for Brother QL and PT label printers")]
struct Cli {
    /// Machine-readable output (one JSON object).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the known printer models.
    Models,
    /// List the label media of a model.
    Media {
        /// Model name ("QL-1100", "PT-P710BT"); default: the connected printer, else QL-1100.
        #[arg(long)]
        model: Option<String>,
    },
    /// Show connected printers and the loaded media.
    Status,
    /// Render a label to a preview PNG (and optionally the raw printer job).
    Render {
        #[command(flatten)]
        label: LabelArgs,
        /// Preview PNG of the whole label.
        #[arg(long, short)]
        out: PathBuf,
        /// Also write the printer job (raw bytes).
        #[arg(long)]
        job: Option<PathBuf>,
    },
    /// Print a label.
    Print {
        #[command(flatten)]
        label: LabelArgs,
        #[arg(long, default_value_t = 1)]
        copies: u32,
        /// Send through this system print queue (raw, no status) instead of USB.
        #[arg(long)]
        queue: Option<String>,
    },
    /// Decode a raw printer job back into a PNG (for debugging).
    Decode {
        job: PathBuf,
        #[arg(long)]
        model: String,
        #[arg(long)]
        media: String,
        #[arg(long, short)]
        out: PathBuf,
    },
}

#[derive(Args)]
struct LabelArgs {
    /// Label text; "\n" starts a new line.
    #[arg(long, short, default_value = "")]
    text: String,
    /// Media key ("62", "62x29", "d24", "24", "hs5.8"); defaults to what the printer has loaded.
    #[arg(long, short)]
    media: Option<String>,
    /// Model name ("QL-1100", "PT-P710BT"); defaults to the connected printer.
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    font: Option<String>,
    /// Font size in points (default: as large as fits).
    #[arg(long)]
    size: Option<f32>,
    #[arg(long)]
    bold: bool,
    #[arg(long)]
    italic: bool,
    /// left, center or right.
    #[arg(long, default_value = "center")]
    align: String,
    /// Run the text across the tape instead of along it (or vice versa for die-cut labels).
    #[arg(long)]
    rotate: bool,
    /// Total length of a continuous label in mm (default: fit the content).
    #[arg(long)]
    length: Option<f32>,
    /// Space around the content in mm.
    #[arg(long, default_value_t = 1.0)]
    padding: f32,
    /// Add a QR code; without a value it encodes the label text.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    qr: Option<String>,
    #[arg(long)]
    frame: bool,
    /// First line larger and bold.
    #[arg(long)]
    heading: bool,
}

impl LabelArgs {
    fn label(&self, media: &Media) -> Result<Label> {
        let align = match self.align.as_str() {
            "left" => Align::Left,
            "center" => Align::Center,
            "right" => Align::Right,
            other => return Err(anyhow!("unknown alignment {other:?} (left, center, right)")),
        };
        let natural = Direction::default_for(media);
        Ok(Label {
            text: self.text.replace("\\n", "\n"),
            font: self.font.clone().unwrap_or_default(),
            bold: self.bold,
            italic: self.italic,
            size_pt: self.size,
            align,
            direction: Some(if self.rotate { natural.flipped() } else { natural }),
            length_mm: self.length,
            padding_mm: self.padding,
            qr: self.qr.is_some(),
            qr_content: self.qr.clone().unwrap_or_default(),
            frame: self.frame,
            heading: self.heading,
        })
    }
}

/// Error that means "the printer is not ready", reported with exit code 2.
#[derive(Debug)]
struct NotReady(String);
impl std::fmt::Display for NotReady {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for NotReady {}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let code = if e.downcast_ref::<NotReady>().is_some() { 2 } else { 1 };
            if cli.json {
                println!("{}", serde_json::json!({ "ok": false, "error": format!("{e:#}"), "code": code }));
            } else {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(code)
        }
    }
}

fn media_by_key(model: &'static Model, key: &str) -> Result<&'static Media> {
    model
        .media_by_key(key)
        .ok_or_else(|| anyhow!("{} has no media {key:?}, see `labelwerk media --model {}`", model.name, model.name))
}

fn model_by_name(name: &str) -> Result<&'static Model> {
    Model::by_name(name).ok_or_else(|| anyhow!("unknown model {name:?}, see `labelwerk models`"))
}

/// What the first printer on USB tells about itself, if anything.
fn connected() -> Option<Probe> {
    let device = transport::list_usb().ok()?.into_iter().next()?;
    UsbPrinter::open(&device).and_then(|mut p| transport::probe(&mut p)).ok()
}

/// The model to work with: named, or the connected printer's, or the QL-1100.
fn pick_model(name: Option<&str>) -> Result<&'static Model> {
    match name {
        Some(name) => model_by_name(name),
        None => Ok(connected().and_then(|p| p.model()).unwrap_or_else(Model::default_model)),
    }
}

/// Model and media for rendering: as named, else what the connected printer has loaded.
fn pick_model_and_media(model: Option<&str>, media: Option<&str>) -> Result<(&'static Model, &'static Media)> {
    let probe = if model.is_none() || media.is_none() { connected() } else { None };
    let model = match model {
        Some(name) => model_by_name(name)?,
        None => probe.as_ref().and_then(|p| p.model()).unwrap_or_else(Model::default_model),
    };
    let media = match media {
        Some(key) => media_by_key(model, key)?,
        None => match &probe {
            Some(Probe::Status(s)) if s.model().is_some_and(|m| m.name == model.name) => s.media(),
            _ => None,
        }
        .unwrap_or(&model.media[0]),
    };
    Ok((model, media))
}

fn open_printer() -> Result<UsbPrinter> {
    let devices = transport::list_usb()?;
    let device = devices.first().ok_or_else(|| NotReady("no Brother label printer on USB".into()))?;
    UsbPrinter::open(device)
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::Models => {
            if cli.json {
                let list: Vec<_> = models()
                    .iter()
                    .map(|m| serde_json::json!({ "name": m.name, "family": m.family, "dpi": m.dpi, "head_pins": m.head_pins, "support": m.protocol.support, "media": m.media.len() }))
                    .collect();
                println!("{}", serde_json::json!({ "ok": true, "models": list }));
            } else {
                for m in models() {
                    println!("{:<12} {:>3} dpi {:>4} pins  {:<10?} {} media", m.name, m.dpi, m.head_pins, m.protocol.support, m.media.len());
                }
            }
        }
        Command::Media { model } => {
            let model = pick_model(model.as_deref())?;
            if cli.json {
                let list: Vec<_> = model
                    .media
                    .iter()
                    .map(|m| serde_json::json!({ "key": m.key(), "label": m.label(), "kind": m.kind, "print_width_dots": m.print_width, "print_length_dots": m.print_length }))
                    .collect();
                println!("{}", serde_json::json!({ "ok": true, "model": model.name, "media": list }));
            } else {
                println!("{} ({} dpi):", model.name, model.dpi);
                for m in &model.media {
                    println!("  {:<8} {}", m.key(), m.label());
                }
            }
        }
        Command::Status => {
            let usb = transport::list_usb()?;
            let mut printers = Vec::new();
            for d in &usb {
                printers.push((d, UsbPrinter::open(d).and_then(|mut p| transport::probe(&mut p))));
            }
            let queues = transport::list_system_queues();
            if cli.json {
                let list: Vec<_> = printers
                    .iter()
                    .map(|(d, s)| match s {
                        Ok(Probe::Status(s)) => serde_json::json!({ "device": d, "model": s.model_name(), "known": s.is_supported_model(), "media": s.media().map(|m| m.key()), "errors": s.errors() }),
                        Ok(Probe::Silent(m)) => serde_json::json!({ "device": d, "model": m.name, "known": true, "media": null, "errors": [], "note": "this model cannot report its status" }),
                        Err(e) => serde_json::json!({ "device": d, "error": format!("{e:#}") }),
                    })
                    .collect();
                println!("{}", serde_json::json!({ "ok": true, "usb": list, "queues": queues }));
            } else {
                if printers.is_empty() {
                    println!("No Brother label printer on USB.");
                }
                for (d, s) in &printers {
                    match s {
                        Ok(Probe::Silent(m)) => println!("{} (USB {}): connected, cannot report its tape", m.name, d.serial.as_deref().unwrap_or("-")),
                        Ok(Probe::Status(s)) => {
                            let media = s.media().map(|m| m.label()).unwrap_or_else(|| "no known media".into());
                            let errors = s.errors();
                            let state = if errors.is_empty() { "ready".to_string() } else { errors.join(", ") };
                            println!("{} (USB {}): {media}, {state}", s.model_name(), d.serial.as_deref().unwrap_or("-"));
                        }
                        Err(e) => println!("{} (USB): {e:#}", d.product),
                    }
                }
                for q in &queues {
                    println!("System queue {} -> {}", q.name, q.uri);
                }
            }
        }
        Command::Render { label, out, job } => {
            let (model, media) = pick_model_and_media(label.model.as_deref(), label.media.as_deref())?;
            let mut renderer = Renderer::new();
            let rendered = renderer.render(&label.label(media)?, model, media);
            std::fs::write(out, preview_png(&rendered, &Default::default())).with_context(|| format!("writing {}", out.display()))?;
            if let Some(job_path) = job {
                std::fs::write(job_path, encode_job(model, media, &[&rendered.page], &PrintOptions::default())?)?;
            }
            let (w_mm, h_mm) = rendered.geometry.label_mm();
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "ok": true, "model": model.name, "media": media.key(), "label_mm": [w_mm, h_mm], "font_pt": rendered.font_pt, "warnings": rendered.warnings, "preview": out })
                );
            } else {
                println!("{} {} label, {:.1} × {:.1} mm -> {}", model.name, media.label(), w_mm, h_mm, out.display());
                if let Some(pt) = rendered.font_pt {
                    println!("font size {pt:.1} pt");
                }
                for w in &rendered.warnings {
                    println!("warning: {w}");
                }
            }
        }
        Command::Print { label, copies, queue } => {
            let mut renderer = Renderer::new();
            let opts = PrintOptions::default();
            if let Some(name) = queue {
                let model = model_by_name(label.model.as_deref().ok_or_else(|| anyhow!("--model is required with --queue"))?)?;
                let media = media_by_key(model, label.media.as_deref().ok_or_else(|| anyhow!("--media is required with --queue"))?)?;
                let queue = transport::list_system_queues()
                    .into_iter()
                    .find(|q| &q.name == name)
                    .ok_or_else(|| NotReady(format!("no supported system queue named {name:?}")))?;
                let rendered = renderer.render(&label.label(media)?, model, media);
                let pages = vec![&rendered.page; *copies as usize];
                transport::print_system_queue(&queue, model, media, &pages, &opts)?;
                report(cli, &format!("sent {copies} label(s) to {}", queue.name));
                return Ok(());
            }
            let mut printer = open_printer()?;
            let probe = transport::probe(&mut printer)?;
            let model = match &probe {
                Probe::Status(s) => s.model().ok_or_else(|| NotReady(format!("{} is not a known QL or PT model", s.model_name())))?,
                Probe::Silent(m) => m,
            };
            let media = match (&label.media, &probe) {
                (Some(key), _) => media_by_key(model, key)?,
                (None, Probe::Status(s)) => s.media().ok_or_else(|| NotReady("the printer reports no known media".into()))?,
                (None, Probe::Silent(m)) => return Err(NotReady(format!("{} cannot report its tape, pass --media", m.name)).into()),
            };
            if let Probe::Status(status) = &probe {
                transport::check_ready(status, model, media).map_err(|e| NotReady(format!("{e:#}")))?;
            }
            let mut confirmed = true;
            let rendered = renderer.render(&label.label(media)?, model, media);
            for w in &rendered.warnings {
                eprintln!("warning: {w}");
            }
            let pages = vec![&rendered.page; *copies as usize];
            transport::print_usb(&mut printer, model, media, &pages, &opts, |event| {
                if !cli.json {
                    match event {
                        PrintEvent::Printing { done, total } if done > 0 => eprintln!("printed {done}/{total}"),
                        PrintEvent::Cooling => eprintln!("print head cooling down, waiting ..."),
                        _ => {}
                    }
                }
                if event == PrintEvent::Unconfirmed {
                    confirmed = false;
                }
            })?;
            let verb = if confirmed { "printed" } else { "sent (the printer cannot confirm)" };
            report(cli, &format!("{verb} {copies} label(s) on {} {}", model.name, media.label()));
        }
        Command::Decode { job, model, media, out } => {
            let model = model_by_name(model)?;
            let media = media_by_key(model, media)?;
            let data = std::fs::read(job)?;
            let pages = decode_job(model, &data)?;
            let first = pages.first().ok_or_else(|| anyhow!("no pages in {}", job.display()))?;
            std::fs::write(out, lines_to_bitmap(model, media, &first.lines).to_png())?;
            report(cli, &format!("{} page(s), first has {} lines -> {}", pages.len(), first.lines.len(), out.display()));
        }
    }
    Ok(())
}

fn report(cli: &Cli, message: &str) {
    if cli.json {
        println!("{}", serde_json::json!({ "ok": true, "message": message }));
    } else {
        println!("{message}");
    }
}
