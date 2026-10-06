# Brother Raster Command Language — Per-Model Protocol Table

Researched 2026-10-06 from Brother's official "Software Developer's Manual – Raster
Command Reference" PDFs (downloaded to `research/docs/*.pdf`, git-ignored; text dumps via
`pdftotext -layout` into `research/docs/*.txt`), cross-checked against open-source drivers
(`pklaus/brother_ql`, `ptouch-print` @ git.familie-radermacher.ch, plus a few smaller
reverse-engineering projects cited inline). **Official PDF wins on disagreement**; driver
source is used to fill gaps where Brother never published a model-specific manual, and is
always flagged as such.

Everything not explicitly sourced below is marked **UNVERIFIED**.

## 0. Legend / cross-cutting rules confirmed from the PDFs

- **Raster line command format differs by family, not by individual model:**
  - **QL family:** lowercase `g` + `0x00` (placeholder "s" byte) + **1-byte** length `n` + data. `67 00 n d1..dn`. (`cv_qlseries_eng_raster_600.pdf` §5 "Raster graphics transfer"; `cv_ql800_eng_raster_101.pdf` §4.)
  - **PT family (all generations incl. the 1990s–2000s "CBP-RASTER" 9xxx line):** uppercase `G` + **2-byte little-endian** length `n1 n2` + data, no placeholder byte. `47 n1 n2 d1..dk`. (`cv_pth500p700e500_eng_raster_111.pdf`, `cv_pte550wp750wp710bt_eng_raster_102.pdf`, `cv_ptp900_eng_raster_102.pdf`, and the 2003 `Brother-PT-9500PC-CBP-Raster-Mode-Command-Reference.pdf`, all §4/§5 "Raster graphics transfer".)
  - This matches `pklaus/brother_ql/raster.py::add_raster_data`: `if self.model.startswith('PT'): write 0x47 + len%256 + len//256 else: write 0x67 0x00 + len`.
- **`Z` (zero raster, `5A`)** only has effect in TIFF/PackBits compression mode — every doc that defines it says "(Valid only when TIFF is selected as the compression mode)" in the print-data overview (`cv_qlseries_eng_raster_600.pdf` p.4; `cv_ql710720_eng_raster_100.pdf` p.6; `cv_ql600710720_eng_raster_102.pdf` p.6). Consequently every model that lacks compression effectively has no usable `Z`, even on docs that don't say so explicitly.
- **Two media-type code families exist for QL**, and they are easy to confuse:
  - Legacy (QL-500…QL-1060N, and — surprisingly — also QL‑1100/1110NWB/1115NWB): continuous = `0x0A`, die‑cut = `0x0B` (`cv_qlseries_eng_raster_600.pdf` §4.2.3; `cv_ql1100_eng_raster_100.pdf` §4(4)).
  - "New" QL status-reply byte (QL‑700‑gen and QL‑800‑gen only): continuous = `0x4A`, die‑cut = `0x4B` in the **status reply**, but the **print‑info command** (`ESC i z` `{n2}`) still uses the old `0x0A`/`0x0B` values even on QL‑800 (`cv_ql710720_eng_raster_100.pdf` §4(4) vs ESC i z section; `cv_ql800_eng_raster_101.pdf` same split). I.e. what you *send* and what the printer *reports* use different numeric spaces on these models.
- **PT media-type codes** (laminated/non-laminated/fabric/heat-shrink/etc.) are consistent across all documented PT raster manuals: `0x00` no media, `0x01` laminated, `0x03` non-laminated, `0x04` fabric, `0x11` heat-shrink 2:1, `0x17` heat-shrink 3:1, `0x13` FLe, `0x14` flexible ID, `0x15` satin, `0xFF` incompatible. Oldest PT family (9500PC "CBP") uses a different, smaller set: `0x00` unknown, `0x01` laminated/stamp/security, `0x02` lettering/iron-on, `0x03` non-laminated/thermal, `0x08` AV tape, `0x09` HG tape.
- **PT status-reply media width is in mm, rounded up to the next integer** — 3.5 mm tape reports `4` (`cv_pth500p700e500_eng_raster_111.pdf` §4.2.2(3)(a) "TZe tape" table).
- **`ESC i z` `{n9}` (page position) has two different value sets** depending on generation:
  - 2-value (older raster family — QL all generations, PT‑H500/P700/E500, PT‑E550W/P750W/P710BT): `0`=starting page, `1`=other pages.
  - 3-value (PT‑P900/P900W/P950NW/P910BT, and reverse-engineered as also used by the PT‑D410/D460BT/D610BT/E310BT/E560BT "D460BT magic" family): `0`=starting, `1`=other, `2`=last page; single-page jobs always send `2`. (`cv_ptp900_eng_raster_102.pdf` §4 ESC i z; confirmed independently in `ptouch-print/src/libptouch.c::ptouch_info_cmd()`, comment "n9 is set to 2 in order to feed the last of the label and properly stop printing".)

---

## 1. Main table

Columns: **Model(s)** · **status bytes 3/4 (series/model)** · **pins / bytes‑per‑row** · **invalidate NULLs** · **compression (TIFF)** · **Z** · **mode switch** · **source**.
See §2–§5 below for the shared ESC i K/M/A/d bit tables and media-type tables referenced by number.

| Model(s) | Series/Model (byte3/4) | Pins / bytes‑row | Invalidate | Compression | `Z` | Mode switch | Source |
|---|---|---|---|---|---|---|---|
| QL‑500 | `0x30`/`0x4F` | 720 / 90 | not specified numerically in this doc (only "send invalid command for appropriate bytes") | No | effectively no (no compression) | **none** — raster mode only, no ESC/P, no `ESC i a` | `cv_qlseries_eng_raster_600.pdf` §4.2,§5 |
| QL‑550 | `0x30`/`0x4F` | 720 / 90 | same as above | No | effectively no | none | same |
| QL‑560 | `0x34`/`0x31` | 720 / 90 | same as above | No | effectively no | none | same |
| QL‑570 | `0x34`/`0x32` | 720 / 90 | same as above | No | effectively no | none | same |
| QL‑580N | `0x34`/`0x33` | 720 / 90 | 200 bytes (flow‑chart §6.1, "In case QL-1050, please send 350 bytes") | Yes (TIFF), USB/serial/LAN | Yes | `ESC i a {n}`: 0=ESC/P, 1=raster(default), 3=P‑touch Template | same |
| QL‑650TD | `0x30`/`0x51` | 720 / 90 | 200 (350 recommended for QL‑1050 specifically, not 650TD) | Yes, **serial interface only** | Yes | `ESC i a {n}`: 0=ESC/P(normal), 1=raster, 2=ESC/P(text) | same |
| QL‑700 | `0x34`/`0x35` | 720 / 90 | 200 | No (excluded from compression list) | effectively no | none (raster only) | same |
| QL‑1050 | `0x30`/`0x50` | 1296 / 162 | **350** (explicitly called out) | Yes | Yes | `ESC i a`: 0/1/3 as QL‑580N | same |
| QL‑1060N | `0x34`/`0x34` | 1296 / 162 | 200 (350 for QL‑1050 only, per doc wording) | Yes | Yes | `ESC i a`: 0/1/3 | same |
| QL‑600 | `0x34`/`0x47` | 720 / 90 | 200 | Doc text doesn't exclude it in the `M` command section, but the worked example explicitly marks compression as "(QL‑710W/QL‑720NW Only)" — treat QL‑600 compression as **unsupported in practice**, UNVERIFIED-exact | likely unusable (tied to compression) | `ESC i a {n}`: 1=raster (**default**), FF=reset to default (QL‑600‑only quirk, send after the job); 0/3 are "QL‑710W/720NW only" | `cv_ql600710720_eng_raster_102.pdf` §2–§4 |
| QL‑710W | `0x34`/`0x36` | 720 / 90 | 200 | Yes | Yes | `ESC i a`: 0(default)/1/3 | `cv_ql600710720_eng_raster_102.pdf` (supersedes the older v1.00 `cv_ql710720_eng_raster_100.pdf`, same bytes) |
| QL‑720NW | `0x34`/`0x37` | 720 / 90 | 200 | Yes | Yes | same as QL‑710W | same |
| QL‑800 | `0x34`/`0x38` | 720 / 90 | **400** (explicit) | **No** ("The QL‑800 does not support the compression mode") | **No** ("The QL‑800 does not support this command") | `ESC i a {n1}`: 1=raster (be sure to switch) | `cv_ql800_eng_raster_101.pdf` §3,§4 |
| QL‑810W | `0x34`/`0x39` | 720 / 90 | 400 | Yes | Yes | `ESC i a`: 1=raster | same |
| QL‑820NWB | `0x34`/`0x41` | 720 / 90 | 400 | Yes | Yes | `ESC i a`: 1=raster | same |
| QL‑1100 | `0x34`/`0x43` | 1296 / 162 | **350** | Yes | Yes | `ESC i a`: 1=raster | `cv_ql1100_eng_raster_100.pdf` §2,§4 |
| QL‑1110NWB | `0x34`/`0x44` | 1296 / 162 | 350 | Yes | Yes | same | same |
| QL‑1115NWB | `0x34`/`0x45` | 1296 / 162 | 350 (doc notes QL‑1115NWB does not support the widest 102/103 mm tape or some die‑cut sizes) | Yes | Yes | same | same |
| PT‑H500 | series `0x30`, model `d`(0x64) | 128 / 16 | **100** (explicit) | Yes | Yes | `ESC i a` not in this doc's command table at all for H500/P700/E500 — printer appears to boot straight in raster mode (no mode-switch command documented for this family) | `cv_pth500p700e500_eng_raster_111.pdf` §3,§4 |
| PT‑P700 | series `0x30`, model `g`(0x67) | 128 / 16 | 100 | Yes | Yes | same (no `ESC i a` documented); open-source drivers send an extra `ESC i a 01` / read-and-discard a status reply for this device anyway (`FLAG_P700_INIT` in ptouch-print, see §6) | same |
| PT‑E500 | series `0x30`, model `e`(0x65) | 128 / 16 | 100 | Yes | Yes | same | same |
| PT‑E550W | series `0x30`, model `f`(0x66) | 128 / 16 | 100 | Yes | Yes | `ESC i a {n1}`: 0=ESC/P, 1=raster, 3=Template | `cv_pte550wp750wp710bt_eng_raster_102.pdf` §3,§4 |
| PT‑P750W | series `0x30`, model `h`(0x68) | 128 / 16 | 100 | Yes | Yes | `ESC i a {n1}`: 0/1/3, same as E550W | same |
| PT‑P710BT | series `0x30`, model **not listed** in the status table at all in this doc (only USB PID `0x20af` given in Appendix A) | 128 / 16 | 100 | Yes | Yes | `ESC i a`: **does not support ESC/P mode or P‑touch Template mode** — raster only; also does **not** support `ESC i A` ("cut each * labels") | same — **and**: *"ESC i S Status information request — the PT‑E550W/PT‑P750W does not support this command"* (confirmed from the rendered PDF page, not a text-extraction artifact — see `/tmp/pte550_p26-26.png` rendered from page 26) |
| PT‑P900 | series `0x30`, model `q`(0x71) | 560 / 70 | 200 | Yes | Yes | `ESC i a`: not restricted, raster required | `cv_ptp900_eng_raster_102.pdf` §3,§4 |
| PT‑P900W | series `0x30`, model `o`(0x69) | 560 / 70 | 200 | Yes | Yes | same | same |
| PT‑P950NW | series `0x30`, model `p`(0x70) | 560 / 70 | 200 | Yes | Yes | same | same |
| PT‑P910BT | series `0x30`, model `x`(0x78) | 560 / 70 | 200 | Yes | Yes | same, but draft-printing bit and high-res bit of `ESC i K` must be 0 (not supported); heat-shrink tube, Fle tape not supported; `ESC i !` required for bi-di status instead of the `PI_RECOVER` flag's implicit behaviour on P900/P900W/P950NW | same; USB PID `0x20c7` |
| PT‑9500PC | series `0x30`, model `J`(0x4A) | 384 / 48 (AV-tape variants: 36/38/39 bytes for AV1789/1957/2067 cassettes) | not given numerically (generic "appropriate number of bytes") | Yes | Yes | `ESC i R {n1}`: 1=raster graphics mode — **different command from `ESC i a`** used by every other PT model | `Brother-PT-9500PC-CBP-Raster-Mode-Command-Reference.pdf` (2003, v1.0) §3–§5 |
| PT‑9600 | UNVERIFIED — no official PDF published | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | Not on the dev index at all; assumed same CBP‑RASTER family as 9500PC/9700PC/9800PCN by product generation, **not independently confirmed** |
| PT‑9700PC | series byte **not verified**; Brother's own dev index marks the *Raster* column "(\*2)" = **"The printer main body supports this command. However, it is not released at the support page."** | UNVERIFIED exact numbers | UNVERIFIED | UNVERIFIED | UNVERIFIED | Hardware raster-capable per Brother, no public command reference | `support.brother.com/.../command/reference/index.html` (PT section, footnote \*2) — raster *hardware support confirmed*, protocol details not public |
| PT‑9800PCN | same `(*2)` footnote as 9700PC | UNVERIFIED exact pin/byte count | invalidate = 200 (per third-party reverse-engineering, see below) | TIFF implemented but disabled by default ("introduces artifacts on some printers") | Not checked | Uses **`ESC i c` (0x1B 0x69 0x63)**, a 5-parameter print-info command, instead of the 10-parameter `ESC i z` used by every consumer PT/QL — confirms the 9xxx industrial line is a distinct raster sub-dialect | `boxine/rasterprynt/rasterprynt/__init__.py::render()` — reverse-engineered from the Windows driver, explicitly citing the PT‑H500/P700/E500 PDF and the ESC/P reference as partial guides; **not an official raster doc for this exact model** |
| PT‑D410 | UNVERIFIED (no official PDF) | 128 / 16 (assumed, `max_px=128`) | 100 (ptouch-print generic `ptouch_init()`) | UNVERIFIED | UNVERIFIED | `ESC i a 01`; driver flags `FLAG_USE_INFO_CMD|FLAG_HAS_PRECUT|FLAG_D460BT_MAGIC` | `ptouch-print/include/ptouch.h` + `src/libptouch.c` (git.familie-radermacher.ch), driver-only, **UNVERIFIED against an official manual — none published** |
| PT‑D450 | UNVERIFIED | 128 / 16 (driver comment: "I'm unsure if print width really is 128px") | 100 | UNVERIFIED | UNVERIFIED | `ESC i a 01`; flag `FLAG_USE_INFO_CMD` only (no D460BT magic, no precut flag) | same, driver-only |
| PT‑D600 | UNVERIFIED | 128 / 16 | 100 | driver sets `FLAG_RASTER_PACKBITS` → compression assumed yes | UNVERIFIED | `ESC i a 01`; driver comment: "reported to work with quirks (premature cutting, max ~73 mm length)" | same, driver-only |
| PT‑D460BT | UNVERIFIED | 128 / 16 | 100 | UNVERIFIED | UNVERIFIED | `ESC i a 01`, plus `FLAG_P700_INIT` (extra status-flush after rasterstart) **and** the D460BT "magic" sequence: after `ESC i z`, send `1B 69 4B 00` (chain enable) and `1B 69 64 01 00 4D 00` (margin/spacing magic — byte 3 must be `0x4D` or the print is corrupted) before raster data | same, driver-only, very detailed reverse-engineering comments in `libptouch.c` |
| PT‑D610BT | UNVERIFIED | 128 / 16 | 100 | UNVERIFIED | UNVERIFIED | same magic sequence as D460BT | same, driver-only |
| PT‑E310BT | UNVERIFIED | 128 / 16 (comment: "3.5/6/9/12/18 mm TZe tested; 5.2/9/11.2 mm HSe not tested") | 100 | UNVERIFIED | UNVERIFIED | same D460BT-style flags, but **no** `FLAG_HAS_PRECUT` | same, driver-only — "added by Christian … otherwise not returning from libusb_bulk_transfer" |
| PT‑E560BT | UNVERIFIED | 128 / 16 | 100, `min_timeout=10`s (needs a longer status-read timeout than the driver default) | UNVERIFIED | UNVERIFIED | same D460BT-style flags | same, driver-only |
| PT‑E720BT | UNVERIFIED — no official manual, not even listed on Brother's command-reference index | reported 64 "dots"/180dpi, max print height 18 mm, max tape 24 mm (numbers look internally inconsistent — 18 mm at 180 dpi ≈ 128 px, not 64; flagged UNVERIFIED) | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | `exoma-ch/brother-printer` `docs/vendor/INDEX.md` comparison table — third-party reverse-engineering notes, not independently re-verified here |
| PT‑E800W / PT‑E800T / PT‑E800TK | UNVERIFIED — not in any driver table found | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | No source found; flag as **UNVERIFIED, likely raster-capable by generation** (same "EDGE" product line as D800W/E920BT) but nothing confirms it |
| PT‑E850TKW | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | No source found |
| PT‑E920BT | UNVERIFIED — "no PT‑E920BT‑specific manual published" (project's own words) | 560 / 70 (**same head as PT‑P900/P910BT**, 36 mm max tape, 360 dpi), USB PID `0x224B` confirmed on hardware via `lsusb` | UNVERIFIED | UNVERIFIED | UNVERIFIED | Advertises "Raster, Mobile SDK" only (no P‑touch Template, no ESC/P) per its User's Guide | `exoma-ch/brother-printer` `docs/vendor/usb-ids.md` + `docs/vendor/INDEX.md` — explicitly uses the **PT‑P900 family raster manual as a documented proxy**, own PID independently confirmed on real hardware (issue #4, 2026-05-28); command-level details (invalidate count, exact `ESC i K` support) **not independently re-verified by this research** |
| PT‑N25BT | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | No source found at all |
| PT‑P300BT | UNVERIFIED (no official PDF; not on Brother's dev index) | 128 / 16 (confirmed: "128px wide image … 16 bytes") | **64 bytes** of `0x00` used by the reverse-engineered driver ("to clear print buffer?" — uncertain purpose, not a documented requirement) | Yes, TIFF/PackBits (`4D 02` observed) | UNVERIFIED | `ESC i a 01` (same as consumer PT family, **not** the `ESC i R` used by the 9500PC doc it was reverse-engineered from) | `gist.github.com/stecman/ee1fd9a8b1b6f0fdd170ee87ba2ddafd` (`_readme.md`, `labelmaker.py`, `labelmaker_encode.py`) — Bluetooth SPP sniffed from the official Android app; cross-checked against `Ircama/PT-P300BT/ptcbp.py` |
| PT‑P715eBT | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | No source found |
| PT‑P910BT | *(see PT‑P900 group above — it is in the same official PDF)* | | | | | | |
| PT‑9500PC/9600/9700PC/9800PCN | *(see rows above — summarized individually)* | | | | | | |
| PT‑3600 | UNVERIFIED | ptouch-print has a **commented-out, unconfirmed** entry: `{0x04f9, 0x200d, "PT-3600", 384, 360, FLAG_RASTER_PACKBITS, 0, 0}` (384 px / 360 dpi, 48 bytes/row — same head class as PT‑9200DX/9500PC) | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | `ptouch-print/src/libptouch.c` — explicitly disabled/commented in the driver, never confirmed on hardware |
| PT‑D800W | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | No raster driver or official command reference found; only a User's Guide (`ptd800w_caneng_ug_03.pdf`, not a command reference) |
| PT‑2430PC | UNVERIFIED (no official command reference found) | 128 / 16 (driver: `max_px=128, dpi=180`) | UNVERIFIED | No PackBits flag set in driver (plain raster only) | UNVERIFIED | No mode-switch flag recorded; driver has `FLAG_HAS_PRECUT`. Comment: "same head width/dpi as PT‑2700; confirmed precut works on real hardware" | `ptouch-print/src/libptouch.c` (PID `0x202d`), driver-only |
| PT‑2700 | UNVERIFIED | 128 / 16 | UNVERIFIED | No (no PackBits flag) | UNVERIFIED | `FLAG_HAS_PRECUT` only | same (PID `0x201f`), driver-only |
| PT‑2730 | UNVERIFIED | 128 / 16 | UNVERIFIED | No | UNVERIFIED | `FLAG_NONE` — driver note: "reported to work with quirks", "was reported to need 48px whitespace within images before content is actually printed" | same (PID `0x2041`), driver-only |
| PT‑18NR / PT‑18R | **Not raster-compatible** | — | — | — | — | — | Standalone 1990s/2000s keyboard label makers (58-key QWERTY, LCD) with their own built-in-font text/PC-connectable protocol, predating the raster command family entirely; not on Brother's raster/ESC‑P command-reference index at all. UNVERIFIED exact protocol, but **confirmed absent from both the raster and ESC/P reference lists** |

---

## 2. Pin / margin geometry by head family

### 2a. QL 720-pin head (90 bytes/row) — QL‑500…QL‑820NWB, QL‑600/700/710W/720NW
(`cv_qlseries_eng_raster_600.pdf` §3.2.5; identical table repeated in `cv_ql710720_eng_raster_100.pdf`/`cv_ql600710720_eng_raster_102.pdf`/`cv_ql800_eng_raster_101.pdf` with two extra die-cut sizes added over time — 29×42mm, 54×29mm, 60×86mm appear only from the QL‑710/720-era doc onward)

| Tape | Left margin pins | Print area pins | Right margin pins |
|---|---|---|---|
| 12 mm | 585 | 106 | 29 |
| 29 mm | 408 | 306 | 6 |
| 38 mm | 295 | 413 | 12 |
| 50 mm | 154 | 554 | 12 |
| 54 mm | 130 | 590 | 0 |
| 62 mm | 12 | 696 | 12 |

### 2b. QL 1296-pin head (162 bytes/row) — QL‑1050/1060N/1100/1110NWB/1115NWB
(`cv_qlseries_eng_raster_600.pdf` §3.2.5; `cv_ql1100_eng_raster_100.pdf` §2 adds 103 mm continuous tape, "QL-1115NWB does not support this tape size")

| Tape | Left margin pins | Print area pins | Right margin pins |
|---|---|---|---|
| 12 mm | 1116 | 106 | 74 |
| 29 mm | 940 | 306 | 50 |
| 38 mm | 827 | 413 | 56 |
| 50 mm | 686 | 554 | 56 |
| 54 mm | 662 | 590 | 44 |
| 62 mm | 544 | 696 | 56 |
| 102 mm | 76 | 1164 | 56 |
| 103 mm (QL‑1100/1110NWB only) | 58 | 1200 | 38 |

### 2c. PT 128-pin head (16 bytes/row) — PT‑H500/P700/E500/E550W/P750W/P710BT, and (UNVERIFIED) all of D410/D450/D460BT/D600/D610BT/E310BT/E560BT/2430PC/2700/2730
(`cv_pth500p700e500_eng_raster_111.pdf` §2.3.5)

| TZe tape | Left margin pins | Print area pins | Right margin pins |
|---|---|---|---|
| 3.5 mm | 52 | 24 | 52 |
| 6 mm | 48 | 32 | 48 |
| 9 mm | 39 | 50 | 39 |
| 12 mm | 29 | 70 | 29 |
| 18 mm | 8 | 112 | 8 |
| 24 mm | 0 | 128 | 0 |

(Matches the task's stated example exactly: 24 mm = 128 printable pins, offset 0; 12 mm = 70 print pins, 29 left/29 right.)

### 2d. PT 560-pin head (70 bytes/row) — PT‑P900/P900W/P950NW/P910BT (and E920BT by proxy, UNVERIFIED)
(`cv_ptp900_eng_raster_102.pdf` §2.3.5 — note the print area is **not symmetric**: left margin is consistently 16 pins narrower than the right margin at every tape width, because the printhead's active 560 px window sits 8 pins off the mechanical head centre)

| TZe tape | Left margin pins | Print area pins | Right margin pins |
|---|---|---|---|
| 3.5 mm | 248 | 48 | 264 |
| 6 mm | 240 | 64 | 256 |
| 9 mm | 219 | 106 | 235 |
| 12 mm | 197 | 150 | 213 |
| 18 mm | 155 | 234 | 171 |
| 24 mm | 112 | 320 | 128 |
| 36 mm | 45 | 454 | 61 |

This asymmetry is independently confirmed by `ptouch-print/src/libptouch.c`'s PT‑P900Wc entry: `pin_offset = -8` with the comment "the print area is centred at pin 272, not the head centre (pin 280) — a constant offset across all tape widths" (45 vs 61 pins → difference 16 → half-offset 8, matching exactly).

### 2e. PT 384-pin head (48 bytes/row) — PT‑9500PC (9600/9700PC/9800PCN assumed by generation, UNVERIFIED)
(`Brother-PT-9500PC-CBP-Raster-Mode-Command-Reference.pdf` §3.2)

| Tape | Offset pins | Print area pins | Unused pins | Bytes |
|---|---|---|---|---|
| 6 mm | 150 | 84 | 150 | 30 |
| 9 mm | 129 | 126 | 129 | 32 |
| 12 mm | 107 | 170 | 107 | 35 |
| 18 mm | 65 | 254 | 65 | 40 |
| 24 mm | 22 | 340 | 22 | 46 |
| 36 mm | 0 | 384 | 0 | 48 |

---

## 3. `ESC i K` (expanded/advanced mode) bit layouts by generation

Bit numbering below follows each PDF's own convention (older docs use a "Bit 7…Bit 0" MSB diagram; newer docs a "1bit…8bit" LSB-first list — both describe the same bit positions, reconciled here LSB→MSB).

| Generation | bit0 | bit1 | bit2 | bit3 | bit4 | bit5 | bit6 | bit7 |
|---|---|---|---|---|---|---|---|---|
| QL legacy (500…1060N) | — | — | — | cut‑at‑end (default ON) | — | — | high‑res 600dpi (570/580N/700 only) | — |
| QL 600/710/720/800‑gen/1100‑gen | two‑colour printing (800‑gen only) | — | — | cut‑at‑end (default ON) | — | — | high‑res 600dpi | — |
| PT‑H500/P700/E500 | — | — | — | no‑chain‑printing | special‑tape (no cutting) | — | — | no‑buffer‑clear‑when‑printing |
| PT‑E550W/P750W/P710BT | — | — | half‑cut (not used on P710BT) | no‑chain‑printing | special‑tape (no cutting) | — | high‑resolution | no‑buffer‑clear‑when‑printing |
| PT‑P900/P900W/P950NW/P910BT | draft printing (P910BT: must be 0) | — | half‑cut | no‑chain‑printing | special‑tape (no cutting) | — | high‑resolution (P910BT: must be 0) | no‑buffer‑clear (not used on P910BT) |
| PT‑9500PC (CBP) | — | — | half‑cut (laminated tape only) | no‑chain‑printing (default ON) | — | label‑end‑cut | high‑res (360×720 dpi) | no‑buffer‑clear (copy printing) |

`ESC i M` ("various mode") is simpler and stable across the whole family: bit6 = auto cut, bit7 = mirror printing (PT only — the QL docs never mention a mirror bit; only "auto cut" bit6 is defined for QL).

`ESC i A` ("cut every N labels") takes a single byte `n` = 1–255 (default 1 = cut every label); on PT‑P900‑family, `0x00` means "do not cut at all". Not supported on QL‑500 (no cutter) or PT‑P710BT.

`ESC i d` (margin/feed amount) — `n1 + n2*256` dots, family-dependent min/max:

| Family | Min | Max | No‑precut min |
|---|---|---|---|
| QL legacy 720-pin | 35 dots (continuous); die-cut fixed at 0 | 1500 dots | — |
| PT‑H500/P700/E500/E550W/P750W/P710BT | 14 dots (2 mm) | 900 dots (127 mm) | 172 dots (24.3 mm) |
| PT‑P900 family, normal (360dpi) | 14 dots (1 mm) | 1800 dots (127 mm) | 382 dots (27 mm) |
| PT‑P900 family, high-res (360×720dpi) | 28 dots (1 mm) | 3600 dots (127 mm) | 764 dots (27 mm) |

---

## 4. Media-type code tables

**QL legacy (status reply, all generations) / `ESC i z` print-info for every QL generation incl. 800/1100:**
`0x00` no media, `0x0A` continuous length tape, `0x0B` die-cut labels.

**QL status-reply "new" codes (QL‑700‑gen and QL‑800‑gen status byte only — NOT the print-info command, NOT QL‑1100‑gen):**
`0x00` no media, `0x4A` continuous length tape, `0x4B` die-cut labels.

**PT consumer family (H500/P700/E500, E550W/P750W/P710BT, P900‑family):**
`0x00` no media, `0x01` laminated, `0x03` non-laminated, `0x04` fabric (P900 family only), `0x11` heat-shrink 2:1, `0x17` heat-shrink 3:1, `0x13` Fle tape (P900 family only), `0x14` flexible ID, `0x15` satin, `0xFF` incompatible.

**PT‑9500PC (CBP) family:**
`0x00` unknown, `0x01` laminated/stamp/security tape, `0x02` lettering/iron-on transfer, `0x03` non-laminated/thermal, `0x08` AV tape, `0x09` HG tape.

---

## 5. Status reply (32 bytes) — fields that vary by model

All documented raster-family printers (QL and PT alike) share the same 32-byte envelope: byte0=`0x80`, byte1=`0x20` (size), byte2=`'B'`, byte3=series code, byte4=model code, byte8/9=error info 1/2, byte10=media width, byte11=media type, byte18=status type, byte19=phase type, byte20/21=phase number, byte22=notification number. This is identical across every official PDF examined (`cv_qlseries_eng_raster_600.pdf` §4.1, `cv_ql800_eng_raster_101.pdf` §4(table), `cv_pth500p700e500_eng_raster_111.pdf` §4.1, `cv_ptp900_eng_raster_102.pdf` §4.1, and the 2003 `Brother-PT-9500PC...pdf` §4.1).

Series/model byte values per model are given in the main table (§1). Full confirmed list for the models explicitly in the task's byte3/4 list:

| Model | byte3 | byte4 |
|---|---|---|
| QL‑500 | `0x30` | `0x4F` |
| QL‑550 | `0x30` | `0x4F` |
| QL‑560 | `0x34` | `0x31` |
| QL‑570 | `0x34` | `0x32` |
| QL‑580N | `0x34` | `0x33` |
| QL‑600 | `0x34` | `0x47` |
| QL‑650TD | `0x30` | `0x51` |
| QL‑700 | `0x34` | `0x35` |
| QL‑710W | `0x34` | `0x36` |
| QL‑720NW | `0x34` | `0x37` |
| QL‑800 | `0x34` | `0x38` |
| QL‑810W | `0x34` | `0x39` |
| QL‑820NWB | `0x34` | `0x41` |
| QL‑1050 | `0x30` | `0x50` |
| QL‑1060N | `0x34` | `0x34` |
| QL‑1100 | `0x34` | `0x43` |
| QL‑1110NWB | `0x34` | `0x44` |
| QL‑1115NWB | `0x34` | `0x45` |

**Every one of these 18 values matches the task's pre-supplied list exactly** — full independent confirmation from the two official PDFs (`cv_qlseries_eng_raster_600.pdf` and `cv_ql600710720_eng_raster_102.pdf` / `cv_ql800_eng_raster_101.pdf` / `cv_ql1100_eng_raster_100.pdf`).

PT model-code bytes (not requested as a fixed list in the prompt, but documented): series byte is `0x30` ('0') for *every* PT model in every official PT raster PDF found (H500/P700/E500/E550W/P750W/P910BT/P900‑family/9500PC). Model-code byte: H500=`0x64`('d'), E500=`0x65`('e'), P700=`0x67`('g'), E550W=`0x66`('f'), P750W=`0x68`('h'), P900=`0x71`('q'), P900W=`0x69`('o'), P950NW=`0x70`('p'), P910BT=`0x78`('x'), PT‑9500PC=`0x4A`('J'). PT‑P710BT has **no row in the status table at all** in its own PDF (only a USB PID is given) — consistent with the finding that it doesn't support the status-request command.

---

## 6. Notable / unusual findings (task item 10)

1. **QL‑800 does not support `Z` or compression** — both explicitly denied in the PDF ("The QL‑800 does not support this command" / "...the compression mode"), unlike its siblings QL‑810W/QL‑820NWB which support both. Confirmed in `cv_ql800_eng_raster_101.pdf` §4.
2. **QL‑500/QL‑550 must use non-compressed raster data** — no `M` compression command, no `ESC i a` mode switch, no `ESC i K` expanded mode; QL‑500 additionally has no auto-cutter at all. Confirmed `cv_qlseries_eng_raster_600.pdf` §5, cross-checked against `brother_ql/models.py` (`compression=False, mode_setting=False, expanded_mode=False, cutting=False` for QL‑500).
3. **PT‑E550W and PT‑P750W do not support the `ESC i S` status-information-request command at all** — confirmed by rendering the actual PDF page (not a `pdftotext` layout artifact — table cell literally reads "*The PT-E550W/PT-P750W does not support this command."* directly under the `ESC i S` row). This is surprising since status polling is normally considered mandatory in the documented flow charts. Source: `cv_pte550wp750wp710bt_eng_raster_102.pdf` p.22, "3. Print Command List" table.
4. **PT‑P710BT does not support `ESC i A`** (cut-every-N-labels) or ESC/P / P‑touch Template modes — raster only, same doc.
5. **PT‑P700 needs a `G` (uppercase, 2-byte length) raster command** — true of every PT model, not P700-specific; the task's framing ("PT-P700 needs 'G'") is really "all PT models use `G`, never lowercase `g`" — see §0.
6. **PT‑9500PC/9600/9700PC/9800PCN use a different print-information command entirely**: `ESC i c` (`1B 69 63`) with only 5 parameters, instead of the `ESC i z` (`1B 69 7A`, 10 parameters) used by every other raster-family printer. Also uses `ESC i R` (`1B 69 52`) instead of `ESC i a` (`1B 69 61`) to enter raster mode. Confirmed from the official 2003 PT‑9500PC PDF and independently reproduced by `boxine/rasterprynt` for PT‑9800PCN (`yield b'\x1bic\x8e\x01\x12\x00\x00'`). This is the single biggest protocol-family split found in this research — the 9xxx industrial/parallel line is raster-*like* but not command-byte-compatible with the QL/consumer‑PT `ESC i z`/`ESC i a` dialect.
7. **Brother's own command-reference index marks PT‑9700PC and PT‑9800PCN's Raster column with footnote "(\*2)"**: *"The printer main body supports this command. However, it is not released at the support page."* — i.e. Brother confirms the hardware is raster-capable but has deliberately not published the manual. This is distinct from "not raster-compatible."
8. **QL‑600 has a unique post-job mode-reset quirk**: `ESC i a FF` ("mode set as default") must be sent *after* the final "print with feeding" command to restore the printer's default state — documented nowhere else in the QL family. (`cv_ql600710720_eng_raster_102.pdf` p.10.)
9. **Same-document internal inconsistency found**: `cv_qlseries_eng_raster_600.pdf`'s own print-data walkthrough (§3.1, job-data table, item 5) says "Set expanded mode: Only used with QL‑570/580N/650TD/700/1050/1060N" (excludes QL‑560), while the formal command definition two pages later (§5, "Set expanded mode") is headed "(QL‑560/570/580N/650TD/700/1050/1060N)" (includes QL‑560). Flagging rather than guessing which is correct.
10. **PT‑D410/D460BT/D610BT need an undocumented "magic" command sequence** beyond the standard raster flow (`ESC i K 00` chain-enable + `ESC i d 01 00 4D 00` margin-magic, byte 3 must be exactly `0x4D` "or the print gets corrupted") — purely a reverse-engineering finding (`ptouch-print/src/libptouch.c`), **no official documentation exists** for this requirement.
11. **PT‑P900 family's asymmetric print window**: the 560-pin head's active print area is offset 8 pins from mechanical centre at every tape width (left margin is always 16 pins narrower than right margin) — confirmed both in the official PDF's per-width table and independently in `ptouch-print`'s `pin_offset = -8` for the PT‑P900Wc entry.
12. **`brother_ql` (pklaus) has no QL‑600 model entry at all**, despite Brother having shipped an official combined raster manual covering it since 2019 (v1.02) — a real gap between the most popular open-source QL driver and Brother's own documentation.
13. **PT‑P900/P910BT's `ESC i z` print-info `{n2}` media-type values (`0x00`/`0x11`/`0x17`/`0x13`/`0xFF`) differ from the status-reply media-type table for the very same printer** (`0x01` laminated / `0x03` non-laminated / etc.) — laminated and non-laminated tape share the single code `0x00` in the *print-info command* on this model family, unlike H500/P700/E500/E550W/P750W where `{n2}` reuses the status-table values (`0x01`/`0x03`) directly. Another send-vs-report asymmetry, this time on the PT side.

---

## 7. TD / RJ / TJ / PJ / MW / VC (one line, per task scope)

RJ and PJ series are documented under their own **"Raster Command Reference"** PDFs (`cv_rj4000_eng_raster_103.pdf`, `cv_pj_eng_raster_130.pdf`) and appear to share the same `g`/`ESC i z`/`ESC i S` family as QL (not independently verified line-by-line here, out of scope); TD and MW series are documented primarily under separate **"ESC/P Command Reference"** PDFs (`cv_td4000_eng_escp_120.pdf`, `cv_td2000_eng_escp_100.pdf`, `cv_mw_eng_escp_312.pdf`), i.e. TD/MW's primary language is ESC/P, not raster (UNVERIFIED whether a raster variant also exists for any TD model); TJ/VC were not checked at all — UNVERIFIED.

---

## 8. Sources

### Official Brother PDFs (downloaded to `research/docs/`, git-ignored)
- `cv_qlseries_eng_raster_600.pdf` — QL‑500/550/560/570/580N/650TD/700/1050/1060N, v6.0 (2011)
- `cv_ql600710720_eng_raster_102.pdf` — QL‑600/710W/720NW, v1.02 (2019) — supersedes `cv_ql710720_eng_raster_100.pdf` v1.00, kept for reference
- `cv_ql800_eng_raster_101.pdf` — QL‑800/810W/820NWB, v1.01
- `cv_ql1100_eng_raster_100.pdf` — QL‑1100/1110NWB/1115NWB, v1.00
- `cv_pth500p700e500_eng_raster_111.pdf` — PT‑H500/P700/E500, v1.11 (2014)
- `cv_pte550wp750wp710bt_eng_raster_102.pdf` — PT‑E550W/P750W/P710BT, v1.02
- `cv_ptp900_eng_raster_102.pdf` — PT‑P900/P900W/P950NW/P910BT, v1.02 (2020)
- `Brother-PT-9500PC-CBP-Raster-Mode-Command-Reference.pdf` — PT‑9500PC, v1.0 (2003), archived copy via archive.stecman.co.nz (original Brother URL dead)

All converted to text via `pdftotext -layout <file>.pdf <file>.txt` for grepping; page 26 of the E550W/P750W/P710BT PDF additionally rendered to PNG via `pdftoppm` to visually confirm the "ESC i S not supported" table cell.

### Brother web index (not a PDF, but authoritative for scope/footnotes)
- `https://support.brother.com/g/s/es/dev/en/command/reference/index.html` (fetched with `?c=us&lang=en&comple=on&redirect=on` to get server-rendered HTML) — saved as `research/docs/index_us.html` / `index2.html`. Confirms exactly which models have a published Raster / ESC-P / P-touch Template manual, and the "(\*2)" footnote for PT‑9700PC/9800PCN.

### Open-source drivers (cross-check / fill-gaps only, flagged inline wherever used)
- `pklaus/brother_ql` — `brother_ql/models.py`, `brother_ql/raster.py` (saved as `research/docs/brother_ql_models.py` / `brother_ql_raster.py`)
- `ptouch-print` (Dominic Radermacher, git.familie-radermacher.ch/linux/ptouch-print.git) — `include/ptouch.h`, `src/libptouch.c`, `src/ptouch-print.c` (saved as `research/docs/ptouch.h` / `libptouch.c` / `ptouch-print.c`) — this is the live/current repo, matches the task's cited flag names (`FLAG_RASTER_PACKBITS`, `FLAG_P700_INIT`, `FLAG_USE_INFO_CMD`, `FLAG_D460BT_MAGIC`) verbatim
- `boxine/rasterprynt` — `rasterprynt/__init__.py` (saved as `research/docs/rasterprynt_init.py`) — PT‑P950NW/PT‑9800PCN network raster printing, reverse-engineered
- `gist.github.com/stecman/...` (`_readme.md`, `labelmaker.py`, `labelmaker_encode.py`, saved as `research/docs/stecman_*`) and `Ircama/PT-P300BT` (`ptcbp.py`, saved as `research/docs/ircama_ptcbp.py`) — PT‑P300BT Bluetooth SPP reverse-engineering
- `exoma-ch/brother-printer` — `docs/vendor/usb-ids.md`, `docs/vendor/INDEX.md` (saved as `research/docs/exoma_usbids.md` / `exoma_index.md`) — PT‑E920BT-specific third-party research, itself built on the PT‑P900 family PDF as a documented proxy

### Not independently verified / no source found at all
PT‑9600, PT‑E800W/T/TK, PT‑E850TKW, PT‑N25BT, PT‑P715eBT, PT‑D800W, PT‑18NR/18R, PT‑3600 (driver entry exists but is commented out/unconfirmed) — see the UNVERIFIED notes in the main table rather than any fabricated numbers.
