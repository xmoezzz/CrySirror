# SPDX-License-Identifier: MPL-2.0
# Names follow Ren'Py's renpy/minstore.py (MIT) for each release.
"""The names every store starts with. The set grows with the release:
6.99 containers and `_`/`_p`; 7.0 `eval`; 7.3 `absolute` and a
checkpointing `print`; 7.5/8.0 `xrange`, `unicode`, `input`; 7.7/8.2
`defaultdict`, `MultiRevertable`, `position`."""
import builtins
import _crysirror_renpy
from renpy.compat import PY2, basestring, bchr, bord, chr, open, pystr, range, round, str, tobytes, unicode

_version = _crysirror_renpy.version()

python_list = _list = builtins.list
python_dict = _dict = builtins.dict
python_object = _object = builtins.object
python_set = _set = builtins.set
_type = builtins.type

from renpy.revertable import RevertableList as __renpy__list__
list = __renpy__list__
from renpy.revertable import RevertableDict as __renpy__dict__
dict = __renpy__dict__
from renpy.revertable import RevertableSet as __renpy__set__
set = __renpy__set__
Set = __renpy__set__
from renpy.revertable import RevertableObject as object
from renpy.revertable import revertable_range as range
from renpy.revertable import revertable_sorted as sorted
import renpy.ui as ui
from renpy.translation import translate_string as __
import renpy
import renpy.exports

renpy = renpy.exports


def _(s):
    return s


def _p(s):
    import re
    lines = [i.strip() for i in s.split("\n")]
    if lines and not lines[0]:
        lines.pop(0)
    if lines and not lines[-1]:
        lines.pop()
    rv = ""
    para = []
    for l in lines:
        if not l:
            rv += " ".join(para) + "\n\n"
            para = []
        elif re.search(r'\{p[^}]*\}$', l):
            para.append(l)
            rv += " ".join(para)
            para = []
        else:
            para.append(l)
    rv += " ".join(para)
    return rv


__all__ = ["Set", "_", "__", "__renpy__dict__", "__renpy__list__", "__renpy__set__", "_dict", "_list", "_object", "_p", "_set", "_type", "dict", "list", "object", "python_dict", "python_list", "python_object", "python_set", "range", "set", "sorted", "ui"]

if _version >= (7, 0):
    from renpy.python import store_eval as eval
    __all__.append("eval")

if _version >= (7, 3):
    from renpy.display.core import absolute
    _print = builtins.print

    def print(*args, **kwargs):
        import renpy.game
        renpy.game.context().force_checkpoint = True
        _print(*args, **kwargs)

    __all__ += ["absolute", "print", "_print"]

if _version >= (7, 4):
    __all__ += ["PY2", "basestring", "bchr", "bord", "open", "str", "tobytes", "unicode"]

if _version >= (7, 5):
    xrange = range

    def input(*args, **kwargs):
        raise Exception("The Python input and raw_input functions do not work with Ren'Py. Please use the renpy.input function instead.")

    raw_input = input
    __all__ += ["input", "raw_input"]

if (_version[0] == 7 and _version >= (7, 7)) or _version >= (8, 2):
    from renpy.revertable import RevertableDefaultDict as __renpy_defaultdict__
    defaultdict = __renpy_defaultdict__
    from renpy.revertable import MultiRevertable
    from renpy.atl import position
    __all__ += ["position"]

del builtins, _crysirror_renpy
