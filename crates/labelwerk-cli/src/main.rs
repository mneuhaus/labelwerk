//! `labelwerk`: print labels on a Brother QL-1100 from the command line.
//!
//! Exit codes: 0 ok, 1 error, 2 printer not ready (no printer, wrong media, cover open, ...).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use labelwerk_core::protocol::{decode_job, lines_to_bitmap};
use labelwerk_core::render::preview_png;
use labelwerk_core::transport::{self, PrintEvent, UsbPrinter};
use labelwerk_core::{Align, Direction, Label, MEDIA, Media, PrintOptions, Renderer, encode_job};

#[derive(Parser)]
#[command(name = "labelwerk", version, about = "Labels for the Brother QL-1100")]
struct Cli {
    /// Machine-readable output (one JSON object).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the supported label media.
    Media,
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
    /// Media key ("62", "62x29", "d24"); defaults to what the printer has loaded.
    #[arg(long, short)]
    media: Option<String>,
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

fn media_by_key(key: &str) -> Result<&'static Media> {
    Media::by_key(key).ok_or_else(|| anyhow!("unknown media {key:?}, see `labelwerk media`"))
}

fn open_printer() -> Result<UsbPrinter> {
    let devices = transport::list_usb()?;
    let device = devices.first().ok_or_else(|| NotReady("no Brother QL printer on USB".into()))?;
    UsbPrinter::open(device)
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::Media => {
            if cli.json {
                let list: Vec<_> = MEDIA
                    .iter()
                    .map(|m| serde_json::json!({ "key": m.key(), "label": m.label(), "kind": m.kind, "print_width_dots": m.print_width, "print_length_dots": m.print_length }))
                    .collect();
                println!("{}", serde_json::json!({ "ok": true, "media": list }));
            } else {
                for m in MEDIA {
                    println!("{:<8} {}", m.key(), m.label());
                }
            }
        }
        Command::Status => {
            let usb = transport::list_usb()?;
            let mut printers = Vec::new();
            for d in &usb {
                let status = UsbPrinter::open(d).and_then(|mut p| p.request_status());
                printers.push((d, status));
            }
            let queues = transport::list_system_queues();
            if cli.json {
                let list: Vec<_> = printers
                    .iter()
                    .map(|(d, s)| match s {
                        Ok(s) => serde_json::json!({ "device": d, "model": s.model_name(), "media": s.media().map(|m| m.key()), "errors": s.errors() }),
                        Err(e) => serde_json::json!({ "device": d, "error": format!("{e:#}") }),
                    })
                    .collect();
                println!("{}", serde_json::json!({ "ok": true, "usb": list, "queues": queues }));
            } else {
                if printers.is_empty() {
                    println!("No Brother QL printer on USB.");
                }
                for (d, s) in &printers {
                    match s {
                        Ok(s) => {
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
            let key = label.media.as_deref().unwrap_or("62");
            let media = media_by_key(key)?;
            let mut renderer = Renderer::new();
            let rendered = renderer.render(&label.label(media)?, media);
            std::fs::write(out, preview_png(&rendered)).with_context(|| format!("writing {}", out.display()))?;
            if let Some(job_path) = job {
                std::fs::write(job_path, encode_job(media, &[&rendered.page], &PrintOptions::default())?)?;
            }
            let (w_mm, h_mm) = rendered.geometry.label_mm();
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "ok": true, "media": media.key(), "label_mm": [w_mm, h_mm], "font_pt": rendered.font_pt, "warnings": rendered.warnings, "preview": out })
                );
            } else {
                println!("{} label, {:.1} × {:.1} mm -> {}", media.label(), w_mm, h_mm, out.display());
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
                let media = media_by_key(label.media.as_deref().ok_or_else(|| anyhow!("--media is required with --queue"))?)?;
                let queue = transport::list_system_queues()
                    .into_iter()
                    .find(|q| &q.name == name)
                    .ok_or_else(|| NotReady(format!("no supported system queue named {name:?}")))?;
                let rendered = renderer.render(&label.label(media)?, media);
                let pages = vec![&rendered.page; *copies as usize];
                transport::print_system_queue(&queue, media, &pages, &opts)?;
                report(cli, &format!("sent {copies} label(s) to {}", queue.name));
                return Ok(());
            }
            let mut printer = open_printer()?;
            let status = printer.request_status()?;
            let media = match &label.media {
                Some(key) => media_by_key(key)?,
                None => status.media().ok_or_else(|| NotReady("the printer reports no known media".into()))?,
            };
            transport::check_ready(&status, media).map_err(|e| NotReady(format!("{e:#}")))?;
            let rendered = renderer.render(&label.label(media)?, media);
            for w in &rendered.warnings {
                eprintln!("warning: {w}");
            }
            let pages = vec![&rendered.page; *copies as usize];
            transport::print_usb(&mut printer, media, &pages, &opts, |event| {
                if !cli.json {
                    match event {
                        PrintEvent::Printing { done, total } if done > 0 => eprintln!("printed {done}/{total}"),
                        PrintEvent::Cooling => eprintln!("print head cooling down, waiting ..."),
                        _ => {}
                    }
                }
            })?;
            report(cli, &format!("printed {copies} label(s) on {}", media.label()));
        }
        Command::Decode { job, media, out } => {
            let media = media_by_key(media)?;
            let data = std::fs::read(job)?;
            let pages = decode_job(&data)?;
            let first = pages.first().ok_or_else(|| anyhow!("no pages in {}", job.display()))?;
            std::fs::write(out, lines_to_bitmap(media, &first.lines).to_png())?;
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
