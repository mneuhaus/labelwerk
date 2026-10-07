# Labelwerk

Label printing for Brother label printers (QL and PT series) without P-touch Editor. Written in Rust, interface
in GPUI (gpui-kit 0.7), for macOS and Windows.

**Status: 0.1.0-alpha.2.** macOS 13+ (Apple Silicon and Intel) and Windows 10/11 (x64; runs on Windows on Arm
through emulation). The interface is in English, or German when the system language is German.
Website: <https://mneuhaus.github.io/labelwerk/>

![Labelwerk](docs/screenshot-dark.png)

- Type the text; the preview shows the label at real size with exactly the layout that gets printed.
  "Print dots" shows the dots the print head actually sets.
- Type: bundled Barlow or any system font, bold/italic, alignment, size automatic ("as big as it fits") or fixed in
  pt; first line as a heading
- Continuous tape: length follows the content or is fixed in mm; text along or across the tape (⌘R / Ctrl+R)
- Extras: QR code (label text or own content), frame, margin
- The printer is detected over USB (model, loaded tape or labels including the tape colour, errors such as "cover
  open"), and the app adjusts itself to it
- "Recently printed" for quick reprints; the state survives a restart
- ⌘P / Ctrl+P prints

## Install

[Releases](https://github.com/mneuhaus/labelwerk/releases) has the app and the command line tool for both systems.

**macOS:** unzip `Labelwerk-<version>-macos.zip` and move the app to Applications. The app is not notarized by
Apple, so macOS refuses the first start: click "Open Anyway" under System Settings → Privacy & Security, or run once

```sh
xattr -dr com.apple.quarantine /Applications/Labelwerk.app
```

**Windows:** unzip `Labelwerk-<version>-windows-x64.zip` and start `Labelwerk.exe`. The program is not code-signed,
so SmartScreen may warn on the first start: "More info" → "Run anyway".

Connect the printer over USB. No Brother driver is needed: Labelwerk talks to the printer directly (macOS: USB,
Windows: the built-in "USB Printing Support" driver). PT printers with an Editor Lite button (PT-P710BT, PT-P750W)
must have Editor Lite switched off (its LED dark), otherwise they show up as a USB drive instead of a printer.

## Build

```sh
./tools/bundle-macos.sh                # dist/Labelwerk.app and dist/labelwerk (CLI) for this Mac
./tools/bundle-macos.sh --universal    # Apple Silicon + Intel, plus the release archives in dist/
open dist/Labelwerk.app
```

Windows: `cargo build --release -p labelwerk-app -p labelwerk-cli` (needs the Windows SDK for gpui's shader
compiler). Releases build the Windows downloads on GitHub Actions (`.github/workflows/release-windows.yml`) and
attach them to the release.

Development: `cargo run -p labelwerk-app` (app) or `cargo run -p labelwerk-cli -- --help`.
Environment: `LABELWERK_STATE=<file>` (separate state for tests), `LABELWERK_THEME=light|dark`,
`LABELWERK_LANG=en|de`, `LABELWERK_CANVAS=graphit|hell|matte` (work surface), `LABELWERK_DEBUG=1` (USB traffic on
stderr). `swift tools/make-icon.swift` draws the app icons.

## CLI

```sh
labelwerk status                         # connected printer, loaded media
labelwerk models                         # every known model with its support level
labelwerk media --model PT-P710BT        # tapes/labels of one model
labelwerk render -t "Cable\nHDMI" --bold -o preview.png [--job job.bin]
labelwerk print  -t "M3 screws" --copies 3
labelwerk decode job.bin --model PT-P710BT --media 24 -o job.png
```

`--json` prints one JSON object; exit codes: 0 ok, 1 error, 2 printer not ready.

## Supported printers

The model and media data (56 QL and PT models, pins, print widths, offsets) come from P-touch Editor's own model
definitions (`tools/import-ptouch.py` → `crates/labelwerk-core/data/models.json`). The protocol per model is in
`crates/labelwerk-core/src/model.rs` and `research/protocol-table.md`.

| Level | Meaning | Models |
|---|---|---|
| Verified | checked byte for byte against Brother's driver | QL-1100 |
| Documented | follows Brother's raster command reference for exactly this model | the other QL models (QL-500 to QL-1115NWB), PT-P710BT (status and printing work on the device, visual check pending), PT-P700/P750W, PT-E500/E550W, PT-H500, PT-P900 series |
| Assumed | same family and print head as documented models | the other PT-D/E/P models, see `labelwerk models` |
| Unsupported | different protocol | PT-9500PC/9600/9700PC/9800PCN/3600, PT-18R/18NR, PT-N25BT |

TD, RJ, TJ, PJ, MW and VC devices speak other protocols and are not included. Reports on which model prints for you
(or doesn't) help a lot: please open an issue with the output of `labelwerk status --json`.

## Layout

```
crates/labelwerk-core   models/media, raster protocol (PackBits, status), renderer, USB and system queue transport
crates/labelwerk-cli    labelwerk (CLI)
crates/labelwerk-app    Labelwerk (GPUI app)
tools/                  import-ptouch.py, bundle-macos.sh, make-icon.swift, winid.swift (window id for screenshots)
research/               notes; Brother's PDFs and reference data stay local in research/docs (not in git)
docs/                   website (GitHub Pages)
```

Printing goes over USB directly, with status, the way P-touch Editor does it: through `nusb` on macOS and Linux,
through the usbprint.sys device interface on Windows. Fallback when USB is busy: the raw job goes to a system queue
(CUPS `lp -o raw`, or the Windows spooler with the RAW datatype).

## Tests

- `cargo test --workspace` (CI runs it on macOS and Windows)
- `crates/labelwerk-core/tests/brother_filter.rs` sends test pages through Brother's macOS driver
  (`rastertobrotherQL1100`) and compares commands and every raster line with Labelwerk's encoder. 22 of 25 QL-1100
  media are byte-identical; for 23×23 mm, 60×86 mm and Ø 12 mm Brother's CUPS driver differs from Brother's own
  reference and P-touch Editor, and Labelwerk follows the reference. Without the driver these tests are skipped.
- PT-P710BT: status and printing on the real device (24 mm TZe) on macOS.

## Open

- Windows printing on real hardware is untested so far (the transport is built from Microsoft's usbprint
  interface; reports welcome)
- Network printers (QL-1110NWB, PT-P750W over TCP 9100) and Bluetooth
- Two-colour printing (QL-8xx), 600/360 dpi high resolution, half cut, chain printing
- Images/logos and barcodes other than QR
- Notarized macOS app, signed Windows build

## License

MIT, see [LICENSE](LICENSE). The bundled fonts Barlow and IBM Plex Mono are under the SIL Open Font License (texts
next to the font files). Labelwerk is not a Brother product; Brother, P-touch and the model names are trademarks of
Brother Industries, Ltd.
