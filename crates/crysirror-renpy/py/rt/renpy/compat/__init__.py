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


def _py2_text(b):
    if not isinstance(b, bytes):
        return b
    try:
        return b.decode("utf-8")
    except UnicodeDecodeError:
        return b.decode("latin-1")


class Py2File(object):
    """A game file as Python 2 code sees it. The Python 2 dialect has one
    text type, so the byte strings Ren'Py 6/7 returns are decoded: UTF-8
    when valid, otherwise Latin-1 (one character per byte)."""

    def __init__(self, f):
        self._f = f

    def read(self, n=-1):
        return _py2_text(self._f.read(n))

    def readline(self, n=-1):
        return _py2_text(self._f.readline(n))

    def readlines(self, hint=-1):
        return [_py2_text(l) for l in self._f.readlines(hint)]

    def __iter__(self):
        return self

    def __next__(self):
        line = self._f.readline()
        if not line:
            raise StopIteration
        return _py2_text(line)

    next = __next__

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self._f.close()

    def __getattr__(self, name):
        return getattr(self._f, name)
