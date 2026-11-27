# SPDX-License-Identifier: MPL-2.0
"""`renpy.config`: the documented defaults of the game's Ren'Py release
(generated per release under `renpy._profiles`)."""
import collections
import _crysirror_renpy

exec("from renpy._profiles.%s import *" % _crysirror_renpy.config_profile())

language_callbacks = collections.defaultdict(list)
basedir = "/"
gamedir = "/game"
renpy_base = "/"
commondir = "/renpy/common"


def init():
    """`config.init()`: binds the defaults that name engine functions.
    Releases before 7.0 bind scene/show/hide/tts at import; rebinding
    them here gives the same values."""
    import renpy.exports
    import renpy.display.tts
    import renpy.audio.audio
    g = globals()
    g["scene"] = renpy.exports.scene
    g["show"] = renpy.exports.show
    g["hide"] = renpy.exports.hide
    g["tts_function"] = renpy.display.tts.default_tts_function
    if "notify" in g:
        g["notify"] = renpy.exports.display_notify
    if "autoreload_functions" in g:
        g["autoreload_functions"] = [
            (r'\.(png|jpg|jpeg|webp|gif|tif|tiff|bmp|avif|svg)$', renpy.exports.flush_cache_file),
            (r'\.(mp2|mp3|ogg|opus|wav)$', renpy.audio.audio.autoreload),
        ]
    if "gl_blend_func" in g:
        # GL blend equation and factor values (renpy.uguu).
        FUNC_ADD, ONE, ONE_MINUS_SRC_ALPHA, ZERO, DST_COLOR, MIN, MAX = 0x8006, 1, 0x0303, 0, 0x0306, 0x8007, 0x8008
        gl_blend_func["normal"] = (FUNC_ADD, ONE, ONE_MINUS_SRC_ALPHA, FUNC_ADD, ONE, ONE_MINUS_SRC_ALPHA)
        gl_blend_func["add"] = (FUNC_ADD, ONE, ONE, FUNC_ADD, ZERO, ONE)
        gl_blend_func["multiply"] = (FUNC_ADD, DST_COLOR, ONE_MINUS_SRC_ALPHA, FUNC_ADD, ZERO, ONE)
        gl_blend_func["min"] = (MIN, ONE, ONE, MIN, ONE, ONE)
        gl_blend_func["max"] = (MAX, ONE, ONE, MAX, ONE, ONE)


def post_init():
    """`config.post_init()` (8.2+): image exceptions follow developer mode."""
    g = globals()
    if g.get("raise_image_exceptions", False) is None:
        g["raise_image_exceptions"] = g.get("developer", False)
    if g.get("raise_image_load_exceptions", False):
        g["raise_image_load_exceptions"] = g.get("developer", False)
