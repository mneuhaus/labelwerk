# Labelwerk

Etiketten drucken auf Brother-Labeldruckern (QL- und PT-Serie), ohne P-touch Editor. Rust, Oberfläche mit
GPUI (gpui-kit 0.7), damit später auch Windows geht.

- Text eintippen, die Vorschau zeigt **exakt die Pixel, die gedruckt werden** (gleicher Renderer für Vorschau und Druck)
- Schrift: Systemschriften, fett/kursiv, Ausrichtung, Größe automatisch („so groß wie passt") oder fest in pt
- Endlosband: Länge passt sich dem Inhalt an oder fest in mm; Text längs oder quer (⌘R)
- Extras: QR-Code (Etikettentext oder eigener Inhalt), Rahmen, Rand
- Drucker wird per USB erkannt (Modell, eingelegtes Band/Etikett, Fehler wie „Deckel offen"), die App stellt sich
  automatisch darauf ein
- „Zuletzt gedruckt" zum schnellen Nachdrucken, Zustand bleibt beim Neustart erhalten
- ⌘P druckt

## Starten

```sh
./tools/bundle-macos.sh          # baut dist/Labelwerk.app und dist/labelwerk (CLI)
open dist/Labelwerk.app
```

Entwicklung: `cargo run -p labelwerk-app` (App) bzw. `cargo run -p labelwerk-cli -- --help`.
Umgebung: `LABELWERK_STATE=<datei>` (eigener Zustand für Tests), `LABELWERK_THEME=light|dark`,
`LABELWERK_DEBUG=1` (USB-Verkehr auf stderr).

## CLI

```sh
labelwerk status                         # angeschlossener Drucker, eingelegtes Medium
labelwerk models                         # alle bekannten Modelle mit Unterstützungsgrad
labelwerk media --model PT-P710BT        # Bänder/Etiketten eines Modells
labelwerk render -t "Kabel\nHDMI" --bold -o vorschau.png [--job job.bin]
labelwerk print  -t "M3 Schrauben" --copies 3
labelwerk decode job.bin --model PT-P710BT --media 24 -o job.png
```

`--json` liefert ein JSON-Objekt; Exit-Codes: 0 ok, 1 Fehler, 2 Drucker nicht bereit.

## Unterstützte Drucker

Die Modell- und Mediendaten (56 QL- und PT-Modelle, Pins, Druckbreiten, Offsets) stammen aus P-touch Editors
eigenen Modelldefinitionen (`tools/import-ptouch.py` → `crates/labelwerk-core/data/models.json`). Das
Protokoll je Modell steht in `crates/labelwerk-core/src/model.rs` und `research/protocol-table.md`.

| Stufe | Bedeutung | Modelle |
|---|---|---|
| Verified | Byte für Byte gegen Brothers Treiber geprüft | QL-1100 (+1110NWB/1115NWB) |
| Documented | nach Brothers Raster-Referenz für genau dieses Modell | PT-P710BT (Druck auf dem Gerät läuft durch, Sichtprüfung offen), weitere: `labelwerk models` |
| Assumed | gleiche Familie und Druckkopf wie dokumentierte Modelle | siehe `labelwerk models` |

TD-, RJ-, TJ-, PJ-, MW- und VC-Geräte sprechen andere Protokolle und sind nicht dabei.

## Aufbau

```
crates/labelwerk-core   Modelle/Medien, Raster-Protokoll (PackBits, Status), Renderer, USB/CUPS-Transport
crates/labelwerk-cli    labelwerk (CLI)
crates/labelwerk-app    Labelwerk (GPUI-App)
tools/                  import-ptouch.py, bundle-macos.sh, winid.swift (Fenster-ID für Screenshots)
research/               Notizen; Brother-PDFs und Referenzdaten liegen lokal in research/docs (nicht im Git)
```

Druckweg: USB direkt über `nusb` (so macht es auch P-touch Editor, mit Statusabfrage). Fallback: rohe Daten an
eine CUPS-Warteschlange (`lp -o raw`), wenn USB belegt ist.

## Prüfung

- `cargo test --workspace`
- `crates/labelwerk-core/tests/brother_filter.rs` schickt Testbilder durch Brothers macOS-Treiber
  (`rastertobrotherQL1100`) und vergleicht Befehle und jede Rasterzeile mit unserem Encoder. 22 von 25
  QL-1100-Medien sind byte-identisch; bei 23×23 mm, 60×86 mm und Ø 12 mm weicht Brothers CUPS-Treiber von
  Brothers eigener Referenz und P-touch Editor ab, dort folgt Labelwerk der Referenz.
- PT-P710BT: Status und Druck auf dem echten Gerät (24 mm TZe).

## Offen

- Windows: `nusb` braucht dort WinUSB; der saubere Weg ist der Spooler (RAW über den installierten Brother-Treiber)
- Netzwerkdrucker (QL-1110NWB, PT-P750W über TCP 9100) und Bluetooth
- Zweifarbdruck (QL-8xx), 600/360-dpi-Hochauflösung, Halbschnitt, Kettendruck
- Bilder/Logos und Barcodes außer QR
