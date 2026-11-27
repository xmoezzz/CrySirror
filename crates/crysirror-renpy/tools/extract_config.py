#!/usr/bin/env python3
"""Extracts `renpy/config.py` default assignments for one Ren'Py release.

Usage: extract_config.py <renpy-git-checkout> <tag> <out.py>

Keeps top-level assignments whose values are literals or simple containers
(the documented defaults). Defaults computed from the environment or from
earlier defaults are evaluated as the player starts them: with no RENPY_*
environment variables. Callables and engine-object defaults are left to
the runtime, which supplies them natively.
"""
import ast, subprocess, sys

repo, tag, out = sys.argv[1:4]
src = subprocess.run(["git", "-C", repo, "show", f"{tag}:renpy/config.py"], capture_output=True, text=True, check=True).stdout
tree = ast.parse(src)
lines = [f"# Generated from Ren'Py {tag} renpy/config.py (MIT). Do not edit.", ""]
names = []
def ok(v):
    try:
        ast.literal_eval(v)
        return True
    except Exception:
        return isinstance(v, (ast.Set, ast.List, ast.Dict, ast.Tuple)) and all(ok(e) for e in ast.walk(v) if isinstance(e, ast.expr) and e is not v and not isinstance(e, (ast.expr_context, ast.Load)) and False) or False
class _Environ(dict):
    pass


class _Os(object):
    environ = _Environ()


SAFE = {"int": int, "bool": bool, "float": float, "str": str, "tuple": tuple, "list": list, "dict": dict, "set": set, "len": len, "max": max, "min": min}
values = {}


def evaluate(node):
    """The value of a default expression with an empty environment, or
    None when it needs the engine."""
    try:
        v = eval(compile(ast.Expression(node), "<config>", "eval"), {"__builtins__": SAFE, "os": _Os}, dict(values))
    except Exception:
        return None
    r = repr(v)
    try:
        if ast.literal_eval(r) == v:
            return (r,)
    except Exception:
        pass
    return None


for node in tree.body:
    if isinstance(node, ast.Assign) and all(isinstance(t, ast.Name) for t in node.targets):
        seg = ast.get_source_segment(src, node)
        if ok(node.value) or isinstance(node.value, ast.Name) and node.value.id in ("None", "True", "False"):
            lines.append(seg)
            names.extend(t.id for t in node.targets)
            try:
                v = ast.literal_eval(node.value)
                for t in node.targets:
                    values[t.id] = v
            except Exception:
                pass
        elif (ev := evaluate(node.value)) is not None:
            target = " = ".join(t.id for t in node.targets)
            lines.append("%s = %s  # %s, without environment overrides" % (target, ev[0], ast.get_source_segment(src, node.value).replace("\n", " ")))
            names.extend(t.id for t in node.targets)
            for t in node.targets:
                values[t.id] = ast.literal_eval(ev[0])
        else:
            lines.append("# runtime-supplied: " + ", ".join(t.id for t in node.targets))
open(out, "w").write("\n".join(lines) + "\n")
print(len(names), "defaults")
