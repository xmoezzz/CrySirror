#!/usr/bin/env python3
"""Lists the script-facing Ren'Py API of one or more releases.

Usage: api_inventory.py <renpy-git-checkout> <out.json> <tag>...

Static text scan only; nothing from the checkout is imported or executed.
Python 2 sources are not parsed by the host interpreter, so module-level
names come from line patterns:

* `exports`: module-level `def`/`class`/assignments and `from x import a, b`
  of renpy/exports.py (or the renpy/exports/ package);
* `store`: the same for renpy/defaultstore.py plus renpy/minstore.py;
* `config`: top-level assignments in renpy/config.py;
* `dotted`: every `renpy.a.b` reference made by renpy/common/*.rpy, which
  ships with the game and therefore defines what the runtime must provide.
"""
import json, re, subprocess, sys

repo, out, tags = sys.argv[1], sys.argv[2], sys.argv[3:]


def git(*args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True).stdout


def show(tag, path):
    return git("show", f"{tag}:{path}")


def files(tag, prefix):
    return [p for p in git("ls-tree", "-r", "--name-only", tag, prefix).split("\n") if p]


TOP = re.compile(r"^(?:def|class)\s+(\w+)|^(\w+)\s*(?:,\s*\w+\s*)*=(?!=)", re.M)
IMPORT = re.compile(r"^from\s+[\w.]+\s+import\s+(\([^)]*\)|[^\n]+)", re.M)


def toplevel(src):
    names = set()
    for m in TOP.finditer(src):
        names.add(m.group(1) or m.group(2))
    for m in IMPORT.finditer(src):
        body = m.group(1).strip("()")
        if "*" in body:
            continue
        for part in body.replace("\\\n", " ").split(","):
            part = part.split("#")[0].strip()
            if not part:
                continue
            bits = part.split()
            names.add(bits[-1])
    return {n for n in names if n and not n.startswith("__")}


DOTTED = re.compile(r"\brenpy(?:\.[A-Za-z_]\w*)+")


def inventory(tag):
    ex = ""
    paths = files(tag, "renpy/exports")
    if paths:
        for p in paths:
            ex += show(tag, p) + "\n"
    else:
        ex = show(tag, "renpy/exports.py")
    store = show(tag, "renpy/defaultstore.py") + "\n" + show(tag, "renpy/minstore.py")
    config = set(re.findall(r"^(\w+)\s*=(?!=)", show(tag, "renpy/config.py"), re.M))
    dotted = set()
    for p in files(tag, "renpy/common"):
        if p.endswith((".rpy", ".rpym")):
            dotted.update(DOTTED.findall(show(tag, p)))
    # renpy.config.* references are checked through `config`.
    return {
        "exports": sorted(toplevel(ex)),
        "store": sorted(toplevel(store)),
        "config": sorted(config),
        "dotted": sorted(dotted),
    }


result = {tag: inventory(tag) for tag in tags}
with open(out, "w") as f:
    json.dump(result, f, indent=0, sort_keys=True)
for tag, inv in result.items():
    print(tag, {k: len(v) for k, v in inv.items()})
