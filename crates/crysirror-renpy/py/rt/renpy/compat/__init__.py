# SPDX-License-Identifier: MPL-2.0
# Names follow Ren'Py's renpy/compat/__init__.py (MIT).
"""Python 2/3 compatibility names. `PY2` describes the game's own Python
dialect, which the interpreter selects per release."""
import builtins
import _crysirror_renpy

PY2 = _crysirror_renpy.py2()
python_open = builtins.open
open = builtins.open


def compat_open(*args, **kwargs):
    return builtins.open(*args, **kwargs)


basestring = getattr(builtins, "basestring", builtins.str)
pystr = builtins.str
unicode = builtins.str
str = builtins.str
chr = builtins.chr
range = builtins.range
round = builtins.round


def bord(b):
    return b if isinstance(b, int) else builtins.ord(b)


def bchr(i):
    return builtins.bytes([i])


def tobytes(s):
    if isinstance(s, builtins.bytes):
        return s
    return s.encode("latin-1")


def add_attribute(obj, name, value):
    pass


__all__ = ["PY2", "open", "basestring", "str", "pystr", "range", "round", "bord", "bchr", "tobytes", "chr", "unicode"]
