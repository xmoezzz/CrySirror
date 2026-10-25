#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Download third-party showcase assets for the AAA demos (ADR 0010).

Assets are NOT committed: each demo's assets/ folder is gitignored. Every file
is pinned by URL and SHA-256, and its license/attribution is written next to it.
"""
import argparse, hashlib, json, pathlib, sys, urllib.request

KHRONOS = "https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models"
POLYHAVEN = "https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr"
RFVP = "https://raw.githubusercontent.com/xmoezzz/rfvp/062fec82033b554b9eb3b5da069518c0c96a354e/win95_painter_demo"
DEMOS = {
    "rfvp-painter": [
        {"file": f"game/{name}", "url": f"{RFVP}/{name}", "license": "MPL-2.0",
         "attribution": "rfvp win95_painter_demo (xmoezzz/rfvp @ 062fec82)"}
        for name in ["painter.hcb", "graph/w95_bucket", "graph/w95_close", "graph/w95_eraser", "graph/w95_pencil"]
    ],
    "aaa-helmet": [
        {"file": "DamagedHelmet.glb", "url": f"{KHRONOS}/DamagedHelmet/glTF-Binary/DamagedHelmet.glb",
         "license": "CC-BY-4.0", "attribution": "Battle Damaged Sci-fi Helmet - PBR by theblueturtle_ (Khronos glTF-Sample-Assets)"},
        {"file": "royal_esplanade_2k.hdr", "url": f"{POLYHAVEN}/2k/royal_esplanade_2k.hdr",
         "license": "CC0-1.0", "attribution": "Royal Esplanade by Greg Zaal, Poly Haven"},
    ],
    "aaa-systems": [
        {"file": "Fox.glb", "url": f"{KHRONOS}/Fox/glTF-Binary/Fox.glb",
         "license": "CC0-1.0 (model) / CC-BY-4.0 (rigging, animation)", "attribution": "Fox by PixelMannen (CC0), rigging and animation by @tomkranis (CC-BY 4.0), glTF by @AsoboStudio and @scurest"},
        {"file": "venice_sunset_2k.hdr", "url": f"{POLYHAVEN}/2k/venice_sunset_2k.hdr",
         "license": "CC0-1.0", "attribution": "Venice Sunset by Greg Zaal, Poly Haven"},
        {"file": "lantern.glb", "local": "examples/engine/assets/lantern.glb",
         "license": "MPL-2.0", "attribution": "CrySirror procedural Blender fixture"},
    ],
    "aaa-fox-sponza": [
        {"dir": "sponza", "github_dir": "KhronosGroup/glTF-Sample-Assets/main/Models/Sponza/glTF",
         "license": "CC-BY-3.0", "attribution": "Sponza by Frank Meinl, Crytek; glTF by Khronos (glTF-Sample-Assets)"},
        {"file": "Fox.glb", "url": f"{KHRONOS}/Fox/glTF-Binary/Fox.glb",
         "license": "CC0-1.0 (model) / CC-BY-4.0 (rigging, animation)", "attribution": "Fox by PixelMannen (CC0), rigging and animation by @tomkranis (CC-BY 4.0), glTF by @AsoboStudio and @scurest"},
        {"file": "venice_sunset_2k.hdr", "url": f"{POLYHAVEN}/2k/venice_sunset_2k.hdr",
         "license": "CC0-1.0", "attribution": "Venice Sunset by Greg Zaal, Poly Haven"},
    ],
    "aaa-sponza": [
        {"dir": "sponza", "github_dir": "KhronosGroup/glTF-Sample-Assets/main/Models/Sponza/glTF",
         "license": "CC-BY-3.0", "attribution": "Sponza by Frank Meinl, Crytek; glTF by Khronos (glTF-Sample-Assets)"},
        {"file": "venice_sunset_2k.hdr", "url": f"{POLYHAVEN}/2k/venice_sunset_2k.hdr",
         "license": "CC0-1.0", "attribution": "Venice Sunset by Greg Zaal, Poly Haven"},
    ],
}

OUTSIDE_TREE = {"rfvp-painter"}

def fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("demo", choices=sorted(DEMOS))
    p.add_argument("--root", default=str(pathlib.Path(__file__).resolve().parent.parent / "examples"))
    a = p.parse_args()
    # Script bytecode (.hcb) must stay outside the source tree (repository policy).
    root = pathlib.Path(a.root)
    if a.demo in OUTSIDE_TREE and a.root == p.get_default("root"):
        root = pathlib.Path(__file__).resolve().parents[1] / "local/demos"
    out = root / a.demo / "assets"
    out.mkdir(parents=True, exist_ok=True)
    manifest = []
    items = []
    for item in DEMOS[a.demo]:
        if "github_dir" not in item:
            items.append(item)
            continue
        owner, repo, ref, path = item["github_dir"].split("/", 3)
        api = f"https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={ref}"
        for entry in json.loads(fetch(api)):
            if entry["type"] == "file":
                items.append({"file": f"{item['dir']}/{entry['name']}", "url": entry["download_url"],
                              "license": item["license"], "attribution": item["attribution"]})
    for item in items:
        target = out / item["file"]
        target.parent.mkdir(parents=True, exist_ok=True)
        if not target.exists():
            if "local" in item:
                target.write_bytes((pathlib.Path(__file__).resolve().parent.parent / item["local"]).read_bytes())
            else:
                print(f"fetch {item['url']}", file=sys.stderr)
                target.write_bytes(fetch(item["url"]))
        digest = hashlib.sha256(target.read_bytes()).hexdigest()
        manifest.append({**item, "sha256": digest, "bytes": target.stat().st_size})
    (out / "THIRD_PARTY.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))
    return 0

if __name__ == "__main__":
    sys.exit(main())
