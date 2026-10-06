"""Extract the model and media tables of Brother's QL and PT printers from an installed P-touch Editor.

Usage: uv run --no-project python -I tools/import-ptouch.py [path/to/P-touch Editor.app] > crates/labelwerk-core/data/models.json

Only facts are kept (codes, pin counts, sizes in dots and 0.1 mm); protocol details live in
crates/labelwerk-core/src/model.rs.
"""

import json
import re
import sys
from pathlib import Path

app = Path(sys.argv[1] if len(sys.argv) > 1 else "/Applications/P-touch Editor.app")
ptd = app / "Contents/Frameworks/BRLBXWrapperMac.framework/Versions/A/Resources/ptd.bundle"
drivers = json.loads((ptd / "driverlist.json").read_text())


def num(v):
    return int(v, 16) if isinstance(v, str) and v.lower().startswith("0x") else int(v)


def kind_of(name, ntype, family):
    if family == "PT":
        return "Continuous"
    if ntype == 0x0A:
        return "Continuous"
    return "Round" if "Dia" in name else "DieCut"


models = []
for d in drivers:
    name = d["name"].removeprefix("Brother ").strip()
    if not re.match(r"^(QL|PT)-", name):
        continue
    family = name[:2]
    data = json.loads((ptd / d["ptd_json"][0]).read_text())
    m = data["Model"]
    resolutions = [int(x) for x in m["wresolutions"].replace(" ", "").split(",")]
    media = []
    for key in sorted(k for k in data if k.startswith("Paper")):
        p = data[key]
        pname = p["szpapername_mm"].strip()
        if re.search(r"x ?[234]$", pname):
            continue  # split printing across several strips
        ntype = num(p["ntype"])
        media.append(
            {
                "id": num(p["npapersize"]),
                "name": pname,
                "kind": kind_of(pname, ntype, family),
                "media_type": ntype,
                "width_tenth_mm": num(p["npaperwidth"]),
                "length_tenth_mm": num(p["npaperlength"]),
                "print_width": num(p["nimageareawidthres"]),
                "print_length": num(p["nimagearealengthres"]),
                "pins_left": num(p["wpinoffsetleft"]),
                "pins_right": num(p["wpinoffsetright"]),
                "offset_x": num(p["nphysicaloffsetx"]),
                "offset_y": num(p["nphysicaloffsety"]),
            }
        )
    feed = d.get("feed") or {}
    models.append(
        {
            "name": name,
            "family": family,
            "series_code": num(m["byseriescode"]),
            "model_code": num(m["bymodelcode"]),
            "head_pins": num(m["dwheadpinnum"]),
            "dpi": resolutions[0],
            "min_margin_dots": feed.get("min"),
            "default_margin_dots": num(m["ndefaultpapermarginres"]) if "ndefaultpapermarginres" in m else feed.get("min"),
            "min_length_tenth_mm": num(m.get("nminpaperlength", "0")),
            "max_length_tenth_mm": num(m.get("nmaxpaperlength", "0")),
            "max_copies": num(m["wmaxcopies"]),
            "media": media,
        }
    )

json.dump({"source": f"P-touch Editor {app.name}", "models": models}, sys.stdout, indent=1, ensure_ascii=False)
print()
