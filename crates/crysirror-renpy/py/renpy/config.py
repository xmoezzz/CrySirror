# SPDX-License-Identifier: MPL-2.0
"""`renpy.config`: the documented defaults of the game's own Ren'Py release
(generated per release under `renpy._profiles`), plus the values upstream
computes at runtime."""
import collections
import _renpy

_profile = _renpy.config_profile()
exec("from renpy._profiles.%s import *" % _profile)

autosave_frequency = 200
debug_image_cache = False
debug_sound = False
raw_tracebacks = False
log_to_stdout = False
use_drawable_resolution = False
drawable_resolution_text = False
draw_virtual_text_box = False
language_callbacks = collections.defaultdict(list)
profile_time = 0.0
gc_print_unreachable = False
touch_keyboard = False
fbo_size = (4096, 4096)
open_file_encoding = False
log_events = False
tts_function = None

# Paths inside the CrySirror asset/save capabilities.
basedir = "/"
renpy_base = "/"
gamedir = "/game"
commondir = "/renpy/common"
logdir = None
savedir = "/saves"
searchpath = ["/game"]
args = []


def _bind_runtime():
    global scene, show, hide, tts_function
    import renpy.exports
    import renpy.display.tts
    scene = renpy.exports.scene
    show = renpy.exports.show
    hide = renpy.exports.hide
    tts_function = renpy.display.tts.default_tts_function
