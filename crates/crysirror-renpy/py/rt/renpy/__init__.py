# SPDX-License-Identifier: MPL-2.0
# Names follow Ren'Py's renpy/__init__.py (MIT).
"""The `renpy` package of CrySirror's native Ren'Py runtime: release
identity of the game's own Ren'Py, platform flags and module order.
Engine modules are native (see crates/crysirror-renpy/src/rt)."""
import sys
from collections import namedtuple
import _crysirror_renpy
from renpy.compat import PY2, basestring, bchr, bord, chr, open, pystr, range, round, str, tobytes, unicode

_object = object

VersionTuple = namedtuple("VersionTuple", ["major", "minor", "patch", "commit"])
version_tuple = VersionTuple(*_crysirror_renpy.version())
version_only = ".".join(str(i) for i in version_tuple)
official = True
nightly = False
version_name = ""
version = "Ren'Py " + version_only
script_version = version_tuple[0] * 1000000 + version_tuple[1] * 1000 + version_tuple[2]
savegame_suffix = "-LT1.save"
bytecode_version = 1

# The game sees a desktop host; CrySirror owns the real platform.
windows = False
macintosh = False
linux = True
android = False
ios = False
emscripten = False
experimental = False
arch = "unknown"
mobile = False
macapp = False
safe_mode_checked = False
autoreload = False
session = {}
backup = None


def update_path():
    return


def plog(level, even, *args):
    return


_MODULES = [
    "renpy.config", "renpy.display", "renpy.object", "renpy.game", "renpy.preferences",
    "renpy.loader", "renpy.parameter", "renpy.ast", "renpy.atl", "renpy.curry", "renpy.color",
    "renpy.easy", "renpy.execution", "renpy.loadsave", "renpy.savelocation", "renpy.persistent",
    "renpy.revertable", "renpy.rollback", "renpy.python", "renpy.script", "renpy.statements",
    "renpy.style", "renpy.substitutions", "renpy.translation", "renpy.display.render", "renpy.display.matrix",
    "renpy.display.displayable", "renpy.display.core", "renpy.display.scenelists",
    "renpy.display.layout", "renpy.display.focus", "renpy.display.screen", "renpy.display.image",
    "renpy.display.transform", "renpy.display.motion", "renpy.display.behavior",
    "renpy.display.transition", "renpy.display.movetransition", "renpy.display.im",
    "renpy.display.imagelike", "renpy.display.anim", "renpy.display.particle",
    "renpy.display.dragdrop", "renpy.display.viewport", "renpy.display.imagemap",
    "renpy.display.predict", "renpy.text.font", "renpy.text.text", "renpy.audio.audio",
    "renpy.audio.music", "renpy.audio.sound", "renpy.ui", "renpy.sl2.slast",
    "renpy.sl2.sldisplayables", "renpy.character", "renpy.exports", "renpy.minstore",
    "renpy.defaultstore", "renpy.log", "renpy.error", "renpy.display.error", "renpy.arguments",
    "renpy.dump", "renpy.warp", "renpy.scriptedit", "renpy.display.presplash",
    "renpy.display.controller", "renpy.display.gesture", "renpy.display.emulator",
    "renpy.display.joystick", "renpy.test", "renpy.test.testast", "renpy.test.testexecution",
    "renpy.lexer", "renpy.parser", "renpy.pyanalysis", "renpy.savetoken", "renpy.lint",
    "renpy.editor", "renpy.memory", "renpy.bootstrap", "renpy.styledata.styleclass",
    "renpy.styledata.stylesets", "renpy.styledata.styleutil", "renpy.text.extras",
    "renpy.text.shader", "renpy.text.textsupport", "renpy.text.texwrap",
    "renpy.display.tts", "renpy.display.video", "renpy.display.minigame",
    "renpy.audio.filter", "renpy.audio.renpysound", "renpy.gl2.gl2shadercache",
    "renpy.gl2.live2d", "renpy.sl2.slproperties", "renpy.sl2.slparser",
    "renpy.translation.generation",
    "renpy.main",
]


def import_all():
    import importlib
    for name in _MODULES:
        importlib.import_module(name)
    # Pre-7.0 pickles name the style class module renpy.styleclass.
    sys.modules["renpy.styleclass"] = sys.modules["renpy.style"]
    post_import()


def post_import():
    """Creates the store from renpy.defaultstore (upstream post_import)."""
    import renpy
    renpy.python.create_store("store")
    renpy.store = sys.modules["store"]
    renpy.exports.store = renpy.store
    sys.modules["renpy.store"] = sys.modules["store"]
    try:
        import subprocess
        sys.modules["renpy.subprocess"] = subprocess
    except ImportError:
        pass
    skip = ("__all__", "__name__", "__doc__", "__package__", "__loader__", "__spec__", "__file__", "__cached__")
    for k, v in list(renpy.defaultstore.__dict__.items()):
        if version_tuple[0] >= 8 and k in skip:
            continue
        renpy.store.__dict__.setdefault(k, v)
    if version_tuple >= (7, 0):
        renpy.store.eval = renpy.defaultstore.eval
    for k, v in list(globals().items()):
        vars(renpy.exports).setdefault(k, v)


def reload_all():
    raise Exception("Reloading the engine is not supported by this runtime.")
