#!/usr/bin/env python3
"""Generates the values of configuration variables a release does not define.

Usage: gen_absent_config.py <renpy-git-checkout> <profiles-dir>

The native runtime implements the newest behaviour and reads configuration
variables that older releases never had. For each release profile
(config_<major>_<minor>.py) this writes absent_<major>_<minor>.py with every
variable that a newer profile defines but this one lacks, valued as upstream
runs a game of that release on the newest Ren'Py:

1. the value 8.6's renpy/common/00compat.rpy `_set_script_version(version)`
   assigns for the release (`config.<name> = <literal>`), else
2. the newest profile's default (upstream applies it to old games too).

Variables defined only by releases older than the newest profile take the
value of the nearest newer release that defines them. Values that are not
literals are left out (the runtime supplies them).
"""
import ast
import os
import re
import subprocess
import sys

repo, profiles = sys.argv[1:3]
NEWEST_TAG = "8.6.1.26111705"
RELEASES = {"6_7": (6, 7, 1), "6_9": (6, 9, 3), "6_10": (6, 10, 2), "6_11": (6, 11, 2), "6_12": (6, 12, 2), "6_13": (6, 13, 12), "6_14": (6, 14, 1), "6_15": (6, 15, 7), "6_16": (6, 16, 5), "6_17": (6, 17, 7), "6_18": (6, 18, 3), "6_99": (6, 99, 12), "7_0": (7, 0, 0), "7_1": (7, 1, 3), "7_2": (7, 2, 2), "7_3": (7, 3, 5), "7_4": (7, 4, 11), "7_5": (7, 5, 3), "7_6": (7, 6, 3), "7_7": (7, 7, 3), "7_8": (7, 8, 7), "8_0": (8, 0, 3), "8_1": (8, 1, 3), "8_2": (8, 2, 3), "8_3": (8, 3, 7), "8_4": (8, 4, 1), "8_5": (8, 5, 3), "8_6": (8, 6, 1)}


def profile_values(name):
    src = open(os.path.join(profiles, "config_%s.py" % name)).read()
    out = {}
    for node in ast.parse(src).body:
        if isinstance(node, ast.Assign) and all(isinstance(t, ast.Name) for t in node.targets):
            try:
                v = ast.literal_eval(node.value)
            except Exception:
                continue
            for t in node.targets:
                out[t.id] = v
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and node.value is not None:
            try:
                out[node.target.id] = ast.literal_eval(node.value)
            except Exception:
                pass
    names = set(out)
    names |= set(re.findall(r"^([a-z_][a-z_0-9]*)\s*(?::[^=\n]*)?=", src, re.M))
    for line in re.findall(r"# runtime-supplied: (.*)", src):
        names |= {x.strip() for x in line.split(",")}
    return out, names


class Recorder(object):
    """Records attribute assignments. `defaults` gives reads of known
    variables (copies, so in-place changes such as `.clear()` count as
    assignments); other reads, calls and items are recorders."""

    def __init__(self, defaults=None):
        object.__setattr__(self, "assigned", {})
        object.__setattr__(self, "defaults", defaults or {})

    def __setattr__(self, name, value):
        self.assigned[name] = value

    def __getattr__(self, name):
        if name in self.defaults:
            import copy
            v = self.assigned[name] = copy.deepcopy(self.defaults[name])
            return v
        return Recorder()

    def __call__(self, *args, **kwargs):
        return Recorder()

    def __getitem__(self, key):
        return Recorder()

    def __setitem__(self, key, value):
        pass

    def __iter__(self):
        return iter(())


def compat_assignments(version, defaults):
    src = subprocess.run(["git", "-C", repo, "show", NEWEST_TAG + ":renpy/common/00compat.rpy"], capture_output=True, text=True, check=True).stdout
    # The first init block holds _compat_versions and _set_script_version.
    lines = []
    for l in src.split("init -1100 python:", 1)[1].splitlines()[1:]:
        if l.strip() and not l.startswith((" ", "\t")):
            break
        lines.append(l[4:] if l.startswith("    ") else l)
    tree = ast.parse("\n".join(lines))
    funcs = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in ("_compat_versions", "_set_script_version")]
    config = Recorder(defaults)
    env = {"config": config, "store": Recorder(), "bubble": Recorder(), "renpy": Recorder(), "__builtins__": __builtins__}
    exec(compile(ast.Module(body=funcs, type_ignores=[]), "00compat", "exec"), env)
    os.environ.pop("RENPY_EXPERIMENTAL", None)
    # Other store namespaces it touches (build, bubble, ...) only record.
    for _ in range(64):
        try:
            env["_set_script_version"](version)
            break
        except NameError as e:
            env[str(e).split("'")[1]] = Recorder()
            config.assigned.clear()
    return config.assigned


def literal(v):
    try:
        r = repr(v)
        return r if ast.literal_eval(r) == v else None
    except Exception:
        return None


order = sorted(RELEASES, key=lambda k: RELEASES[k])
values = {k: profile_values(k) for k in order}
newest = order[-1]
for name in order:
    vals, names = values[name]
    compat = compat_assignments(RELEASES[name] + (99,), values[newest][0])
    out = ["# Generated by tools/gen_absent_config.py from Ren'Py %s renpy/common/00compat.rpy" % NEWEST_TAG,
           "# and the release profiles (MIT). Do not edit.", ""]
    candidates = set()
    for other in order:
        candidates |= values[other][1]
    for var in sorted(candidates - names):
        if var in compat:
            v, how = compat[var], "00compat for %d.%d" % RELEASES[name][:2]
        elif var in values[newest][0]:
            v, how = values[newest][0][var], "%s default" % newest.replace("_", ".")
        else:
            newer = [o for o in order if RELEASES[o] > RELEASES[name] and var in values[o][0]]
            if not newer:
                continue
            v, how = values[newer[0]][0][var], "%s default" % newer[0].replace("_", ".")
        r = literal(v)
        if r is None:
            out.append("# not literal: %s (%s)" % (var, how))
            continue
        out.append("%s = %s  # %s" % (var, r, how))
    path = os.path.join(profiles, "absent_%s.py" % name)
    open(path, "w").write("\n".join(out) + "\n")
    print(path, len(out) - 3)
