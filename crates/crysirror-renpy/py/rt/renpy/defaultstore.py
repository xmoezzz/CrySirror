# SPDX-License-Identifier: MPL-2.0
# Names follow Ren'Py's renpy/defaultstore.py (MIT), 6.99 through 8.x.
"""The initial contents of the `store` namespace: engine variables,
displayable/transition/character constructors and the `adv` kind."""
from renpy.minstore import *
import renpy
import renpy.display.im as im
import renpy.display.anim as anim
import renpy.display.matrix


def _optional(module, name):
    """A constructor whose subsystem is not part of CrySirror's runtime
    (for example Live2D) raises when used, naming what is missing."""
    try:
        m = __import__(module, fromlist=[name])
        return getattr(m, name)
    except (ImportError, AttributeError):
        def missing(*args, **kwargs):
            raise Exception("%s.%s is not available in this runtime." % (module, name))
        missing.__name__ = name
        return missing


_restart = None
_return = None
_args = None
_kwargs = None
_window = False
_window_subtitle = ''
_rollback = True
_begin_rollback = True
_skipping = True
_dismiss_pause = True
_config = renpy.config
_widget_by_id = None
_widget_properties = {}
_text_rect = None
_menu = False
main_menu = False
_autosave = True
_live2d_fade = True


class _Config(object):

    def __getstate__(self):
        return None

    def __setstate__(self, data):
        return

    def __getattr__(self, name):
        cvars = vars(_config)
        if name not in cvars:
            raise Exception('config.%s is not a known configuration variable.' % (name))
        return cvars[name]

    def __setattr__(self, name, value):
        cvars = _config.__dict__
        if name not in cvars and renpy.config.locked:
            raise Exception('config.%s is not a known configuration variable.' % (name))
        if name == "script_version":
            renpy.store._set_script_version(value)
        if name == "developer":
            if value == "auto":
                renpy.config.original_developer = value
                renpy.config.developer = renpy.config.default_developer
                return
        cvars[name] = value

    def __delattr__(self, name):
        if renpy.config.locked:
            raise Exception('Deleting configuration variables is not supported.')
        else:
            delattr(renpy.config, name)


style = None
config = _Config()
library = config
eval = renpy.python.py_eval

Bar = renpy.display.behavior.Bar
Button = renpy.display.behavior.Button
ImageButton = renpy.display.behavior.ImageButton
Input = renpy.display.behavior.Input
TextButton = renpy.display.behavior.TextButton
ImageReference = renpy.display.image.ImageReference
DynamicImage = renpy.display.image.DynamicImage
Image = renpy.display.im.image
Frame = renpy.display.imagelike.Frame
Borders = renpy.display.imagelike.Borders
Solid = renpy.display.imagelike.Solid
FileCurrentScreenshot = renpy.display.imagelike.FileCurrentScreenshot
LiveComposite = renpy.display.layout.LiveComposite
LiveCrop = renpy.display.layout.LiveCrop
LiveTile = renpy.display.layout.LiveTile
Composite = renpy.display.layout.Composite
Crop = renpy.display.layout.Crop
Tile = renpy.display.layout.Tile
Flatten = renpy.display.layout.Flatten
Null = renpy.display.layout.Null
Window = renpy.display.layout.Window
Viewport = renpy.display.viewport.Viewport
DynamicDisplayable = renpy.display.layout.DynamicDisplayable
ConditionSwitch = renpy.display.layout.ConditionSwitch
ShowingSwitch = renpy.display.layout.ShowingSwitch
AlphaMask = renpy.display.layout.AlphaMask
Layer = _optional("renpy.display.layout", "Layer")
Transform = renpy.display.transform.Transform
Camera = _optional("renpy.display.transform", "Camera")
Animation = anim.Animation
Movie = _optional("renpy.display.video", "Movie")
Particles = renpy.display.particle.Particles
SnowBlossom = renpy.display.particle.SnowBlossom
Text = renpy.text.text.Text
ParameterizedText = _optional("renpy.text.extras", "ParameterizedText")
FontGroup = renpy.text.font.FontGroup
Drag = renpy.display.dragdrop.Drag
DragGroup = renpy.display.dragdrop.DragGroup
Sprite = renpy.display.particle.Sprite
SpriteManager = renpy.display.particle.SpriteManager
Matrix = renpy.display.matrix.Matrix2D if not hasattr(renpy.display.matrix, "Matrix") else renpy.display.matrix.Matrix
Live2D = _optional("renpy.gl2.live2d", "Live2D")
Model = _optional("renpy.display.model", "Model")

_curry = renpy.curry.curry
Alpha = _curry(renpy.display.layout.Alpha)
Position = _curry(renpy.display.layout.Position)
Pan = _curry(renpy.display.motion.Pan)
Move = _curry(renpy.display.motion.Move)
Motion = _curry(renpy.display.motion.Motion)
Revolve = _curry(renpy.display.motion.Revolve)
Zoom = _curry(renpy.display.motion.Zoom)
RotoZoom = _curry(renpy.display.motion.RotoZoom)
FactorZoom = _curry(renpy.display.motion.FactorZoom)
SizeZoom = _curry(renpy.display.motion.SizeZoom)
Fade = _curry(renpy.display.transition.Fade)
Dissolve = _curry(renpy.display.transition.Dissolve)
ImageDissolve = _curry(renpy.display.transition.ImageDissolve)
AlphaDissolve = _curry(renpy.display.transition.AlphaDissolve)
CropMove = _curry(renpy.display.transition.CropMove)
PushMove = _curry(renpy.display.transition.PushMove)
Pixellate = _curry(renpy.display.transition.Pixellate)
OldMoveTransition = _curry(renpy.display.movetransition.OldMoveTransition)
MoveTransition = _curry(renpy.display.movetransition.MoveTransition)
MoveFactory = _curry(renpy.display.movetransition.MoveFactory)
MoveIn = _curry(renpy.display.movetransition.MoveIn)
MoveOut = _curry(renpy.display.movetransition.MoveOut)
ZoomInOut = _curry(renpy.display.movetransition.ZoomInOut)
RevolveInOut = _curry(renpy.display.movetransition.RevolveInOut)
MultipleTransition = _curry(renpy.display.transition.MultipleTransition)
ComposeTransition = _curry(renpy.display.transition.ComposeTransition)
Pause = _curry(renpy.display.transition.NoTransition)
SubTransition = _curry(renpy.display.transition.SubTransition)
del _curry

ADVSpeaker = ADVCharacter = renpy.character.ADVCharacter
Speaker = Character = renpy.character.Character
DynamicCharacter = renpy.character.DynamicCharacter
MultiPersistent = renpy.persistent.MultiPersistent
Action = renpy.ui.Action
BarValue = renpy.ui.BarValue
AudioData = _optional("renpy.audio.audio", "AudioData")
Style = renpy.style.Style
SlottedNoRollback = renpy.rollback.SlottedNoRollback
NoRollback = renpy.rollback.NoRollback


class _layout_class(__builtins__["object"] if isinstance(__builtins__, dict) else __builtins__.object):

    def __init__(self, cls, doc, nargs=0, **extra_kwargs):
        self.cls = cls
        self.nargs = nargs
        self.extra_kwargs = extra_kwargs
        self.__doc__ = doc

    def __call__(self, *args, **properties):
        conargs = args[:self.nargs]
        kids = args[self.nargs:]
        kwargs = self.extra_kwargs.copy()
        kwargs.update(properties)
        rv = self.cls(*conargs, **kwargs)
        for i in kids:
            rv.add(renpy.easy.displayable(i))
        return rv


Fixed = _layout_class(renpy.display.layout.MultiBox, "", layout="fixed")
HBox = _layout_class(renpy.display.layout.MultiBox, "", layout='horizontal')
VBox = _layout_class(renpy.display.layout.MultiBox, "", layout='vertical')
Grid = _layout_class(renpy.display.layout.Grid, "", nargs=2, layout='vertical')


def AlphaBlend(control, old, new, alpha=False):
    return renpy.display.transition.AlphaDissolve(control, 0.0, old_widget=old, new_widget=new, alpha=alpha)


def At(d, *args):
    rv = renpy.easy.displayable(d)
    for i in args:
        if isinstance(i, renpy.display.motion.Transform):
            rv = i(child=rv)
        else:
            rv = i(rv)
    return rv


Color = renpy.color.Color
color = renpy.color.Color
menu = renpy.exports.display_menu
predict_menu = renpy.exports.predict_menu
default_transition = None
mouse_visible = True
suppress_overlay = False

adv = ADVCharacter(None,
                   who_prefix='',
                   who_suffix='',
                   what_prefix='',
                   what_suffix='',
                   show_function=renpy.exports.show_display_say,
                   predict_function=renpy.exports.predict_show_display_say,
                   condition=None,
                   dynamic=False,
                   image=None,
                   interact=True,
                   slow=True,
                   slow_abortable=True,
                   afm=True,
                   ctc=None,
                   ctc_pause=None,
                   ctc_timedpause=None,
                   ctc_position="nestled",
                   all_at_once=False,
                   with_none=None,
                   callback=None,
                   type='say',
                   advance=True,
                   retain=False,
                   who_style='say_label',
                   what_style='say_dialogue',
                   window_style='say_window',
                   screen='say',
                   mode='say',
                   voice_tag=None,
                   kind=False)


def predict_say(who, what):
    who = Character(who, kind=adv)
    try:
        who.predict(what)
    except Exception:
        pass


def say(who, what, interact=True, *args, **kwargs):
    who = Character(who, kind=adv)
    who(what, *args, interact=interact, **kwargs)


_last_say_who = None
_last_say_what = None
_last_say_args = ()
_last_say_kwargs = {}
_cache_pin_set = set()
_predict_set = set()
_predict_screen = dict()
_overlay_screens = None
_in_replay = None
_side_image_attributes = None
_side_image_attributes_reset = False
main_menu = False
_ignore_action = None
_quit_slot = None
_screenshot_pattern = None

import sys
import os

globals()["renpy"] = renpy.exports
del _optional
