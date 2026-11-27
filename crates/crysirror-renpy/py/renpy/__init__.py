# SPDX-License-Identifier: MPL-2.0
"""Script-visible entry points for CrySirror's Ren'Py adapter."""

import _renpy
import builtins
import os
from _renpy_style import Style, StyleManager
import store
from renpy import statements
from renpy.pyanalysis import const, not_const, pure
from renpy.actions import Action, BarValue
from renpy.transitions import Dissolve
from renpy.preferences import register_persistent
from renpy.persistent import Persistent
from renpy.displayables import Displayable, Image, ImageReference, Text, ParameterizedText, Null, Solid, Color, im, register_sl_displayable, register_sl_statement
from renpy import config, object
from renpy import music, sound

atl_warper = _renpy.atl_warper
register_statement = statements.register
loadable = _renpy.loadable
context = _renpy.context
has_label = _renpy.has_label
dynamic = _renpy.dynamic
context_dynamic = _renpy.context_dynamic
call_in_new_context = _renpy.call_in_new_context
in_rollback = _renpy.in_rollback
block_rollback = _renpy.block_rollback
suspend_rollback = _renpy.suspend_rollback
roll_forward_info = _renpy.roll_forward_info


class RollbackException(Exception):
    pass


def can_rollback():
    return config.rollback_enabled and store._rollback and _renpy.rollback_available() > 0


def rollback(force=False, checkpoints=1, defer=False, greedy=True, label=None, abnormal=True):
    if force or defer or not greedy or label is not None or not abnormal:
        raise NotImplementedError("special Ren'Py rollback policy")
    if checkpoints > 0 and can_rollback():
        raise RollbackException(min(checkpoints, _renpy.rollback_available()))
has_screen = _renpy.has_screen
restart_interaction = _renpy.restart_interaction


def show_screen(name, *args, **kwargs):
    return _renpy.show_screen(name, args, kwargs)


def hide_screen(name):
    return _renpy.hide_screen(name)


def call_screen(name, *args, **kwargs):
    return _renpy.call_screen(name, args, kwargs)


def _screen_action_sensitive(action):
    if isinstance(action, (list, tuple)):
        return all(_screen_action_sensitive(item) for item in action)
    return getattr(action, "get_sensitive", lambda: True)()


def _invoke_screen_action(action):
    if isinstance(action, (list, tuple)):
        result = None
        for item in action:
            value = _invoke_screen_action(item)
            if value is not None:
                result = value
        return result
    return action()


class Return(Action):
    def __init__(self, value=True):
        self.value = value

    def __call__(self):
        return self.value


class Jump(Action):
    def __init__(self, label):
        self.label = label

    def __call__(self):
        jump(self.label)


class Curry:
    def __init__(self, function, *args, **kwargs):
        self.function = function
        self.args = args
        self.kwargs = kwargs

    def __call__(self, *args, **kwargs):
        options = dict(self.kwargs)
        options.update(kwargs)
        return self.function(*(self.args + args), **options)

    def __eq__(self, other):
        return (isinstance(other, Curry) and self.function == other.function
                and self.args == other.args and self.kwargs == other.kwargs)


def curry(function):
    return Curry(Curry, function)


def partial(function, *args, **kwargs):
    return Curry(function, *args, **kwargs)


def curried_call_in_new_context(label, *args, **kwargs):
    return partial(call_in_new_context, label, *args, **kwargs)


image = _renpy.image


def showing(name, layer="master"):
    return _renpy.showing(name, layer)


def scene(layer="master"):
    return _renpy.scene(layer)


def show(name, at_list=(), layer=None, what=None, zorder=None, tag=None,
         behind=(), atl=None, transient=False, munge_name=True):
    if transient or not munge_name:
        raise NotImplementedError("special show lifetime")
    return _stmt_show(name, what, tag, at_list, layer, zorder, behind, atl)


def hide(name, layer=None):
    if isinstance(name, str):
        name = name.split()
    if name:
        _renpy.hide(name[0], layer)


def write_log(message, *args):
    if config.log_enable:
        _renpy.write_log(message % args if args else message)


def list_files(common=False):
    return _renpy.list_files(common)


def has_image(name, exact=False):
    return _renpy.has_image(name, exact)


class Context:
    def __init__(self):
        self._menu = False
        self._main_menu = False
        self.force_checkpoint = False


class JumpException(Exception):
    pass


class JumpOutException(Exception):
    pass


class CallException(Exception):
    pass


def show_display_say(who, what, who_args=None, what_args=None, window_args=None,
                     image=False, side_image=None, two_window=False,
                     two_window_vbox_properties=None, who_window_properties=None,
                     say_vbox_properties=None, transform=None, variant=None,
                     screen=None, layer=None, **kwargs):
    if not isinstance(what, str):
        raise NotImplementedError("non-string dialogue")
    if screen and has_screen(screen):
        if image or side_image is not None or two_window or transform is not None:
            raise NotImplementedError("special dialogue displayables")
        show_screen(screen, _transient=True, who=who, what=what, **kwargs)
    return Text(what, **(what_args or {}))


_missing = object.Sentinel("character argument omitted")


class ADVCharacter:
    def __init__(self, name=_missing, kind=None, **properties):
        self.name = (None if kind is None else kind.name) if name is _missing else name
        self.properties = {} if kind is None else dict(kind.properties)
        self.properties.update(properties)

    def __call__(self, text, interact=None, **kwargs):
        options = dict(self.properties)
        options.update(kwargs)
        if interact is None:
            interact = options.get("interact", True)
        condition = options.get("condition")
        if condition is not None and not eval(condition, store.__dict__):
            return None
        name = self.name
        if options.get("dynamic", False) and name is not None:
            name = eval(name, store.__dict__)
        if name is not None:
            name = options.get("who_prefix", "") + str(name) + options.get("who_suffix", "")
        text = options.get("what_prefix", "") + text + options.get("what_suffix", "")
        if interact:
            screen = options.get("screen", "say")
            show = options.get("show_function", show_display_say)
            if show is not show_display_say:
                text_widget = show(name, text)
                if text_widget is not None:
                    text = text_widget.text
            elif screen and has_screen(screen):
                show(name, text, screen=screen)
                return _renpy.say(name, text, screen)
            return _renpy.say(name, text)


def Character(name=_missing, kind=None, **properties):
    if kind is None:
        kind = store.adv
    return type(kind)(name, kind=kind, **properties)


def jump(label):
    raise JumpException(label)


def jump_out_of_context(label):
    raise JumpOutException(label)


def _parent_jump(label):
    return JumpException(label)


def call(label, *args, **kwargs):
    error = CallException()
    error.label = label
    error.args = args
    error.kwargs = kwargs
    raise error


def pause(delay=None, music=None, with_none=None, hard=False, checkpoint=None):
    if music is not None or checkpoint is True:
        raise NotImplementedError("audio-dependent pauses and rollback checkpoints")
    return _renpy.pause(delay, hard)


def _install_persistent(value):
    store.persistent = value
    store._preferences = store.preferences = value._preferences


def save_persistent():
    return _renpy.save_persistent(store.persistent)


def _bootstrap(version, platform):
    global windows, macintosh, linux, android, ios, mobile, macapp
    windows = (10, 0) if platform == "windows" else False
    macintosh = platform == "macos"
    linux = platform == "linux"
    android = platform == "android"
    ios = platform == "ios"
    mobile = android or ios
    macapp = False
    store.renpy = __import__("renpy")
    config.basedir = "/"
    config.renpy_base = "/"
    config.gamedir = "/game"
    config.commondir = "/renpy/common"
    config.savedir = "/saves"
    store.config = config
    store.NoRollback = object.NoRollback
    store._rollback = True
    store.store = store
    store.ui = ui
    _install_persistent(Persistent())
    store.Action = Action
    store.Return = Return
    store.Jump = Jump
    store.BarValue = BarValue
    store.style = StyleManager()
    store.Style = Style
    store.Displayable = Displayable
    store.Null = Null
    store.Dissolve = Dissolve
    store.Solid = Solid
    store.Image = Image
    store.ImageReference = ImageReference
    store.Text = Text
    store.ParameterizedText = ParameterizedText
    store.Color = Color
    store.im = im
    store.ADVCharacter = ADVCharacter
    store.adv = ADVCharacter(None)
    store.Character = Character
    store.DynamicCharacter = DynamicCharacter
    store.python_list = store._list = list
    store.python_dict = store._dict = dict
    store.python_set = store._set = set
    store.python_object = store._object = builtins.object
    store._type = type
    store.__renpy__list__ = list
    store.__renpy__set__ = set
    store.__renpy__dict__ = dict
    store._ = _
    store.__ = translate_string
    store._args = None
    store._kwargs = None
    store._return = None


def DynamicCharacter(name, **kwargs):
    return Character(name, dynamic=True, **kwargs)


from renpy import character, easy, ui
displayable = easy.displayable


_strings = {}


def _(text):
    return text


def translate_string(text):
    return _strings.get((_translate_language(), text), text)


def _translate_language():
    return store._preferences.language


def _stmt_translate_string(language, old, new):
    _strings[(language, old)] = new


def _init_system_styles():
    initialize = getattr(config, "init_system_styles", None)
    if initialize is not None:
        initialize()


def _post_init():
    pass


def _new_context_object(rollback, init_phase):
    value = Context()
    value.rollback = rollback
    value.init_phase = init_phase
    return value


def _stmt_label(name):
    callback = config.label_callback
    if callback is not None:
        callback(name, _renpy.label_entry())


def _stmt_say(who, text, interact, attributes, temporary_attributes, args, kwargs, transition, who_name):
    if transition is not None:
        _stmt_with(transition, None)
    if callable(who):
        return who(text, *args, interact=interact, **kwargs)
    if interact:
        return _renpy.say(who, text)


def say(who, what, interact=True):
    return _stmt_say(who, what, interact, None, None, (), {}, None, None)


def _stmt_menu(entries, selection, transition, caption, args, kwargs):
    if selection is not None:
        raise NotImplementedError("menu set")
    if transition is not None:
        _stmt_with(transition, None)
    return _renpy.menu(entries)


def _stmt_image(name, image):
    return _renpy.image(name, image)


def _stmt_show(name, value, tag, transforms, layer, zorder, behind, atl):
    return _renpy.show(name, value, tag, transforms or (), layer, zorder, behind or (), atl)


def _stmt_hide(name, value, tag, transforms, layer, zorder, behind):
    return _renpy.hide(tag or name[0], layer)


def _stmt_scene(name, value, tag, transforms, layer, zorder, behind, atl, clear_layer):
    _renpy.scene(clear_layer or layer)
    if name is not None:
        _stmt_show(name, value, tag, transforms, layer, zorder, behind, atl)


def _stmt_show_layer(layer, transforms, atl):
    raise NotImplementedError("layer transforms")


def _stmt_with(transition, paired):
    return with_statement(transition, paired=paired)


def with_statement(transition, always=False, paired=None, clear=True):
    if _renpy.in_init():
        raise Exception("with statement during init")
    if config.skipping or not (always or store._preferences.transitions):
        transition = None
    if transition is None:
        return _renpy.dissolve(0)
    if isinstance(transition, Dissolve) and transition.time_warp is None and not transition.alpha:
        return _renpy.dissolve(transition.time)
    raise NotImplementedError("Ren'Py transition object")


def _get_store(name):
    target = store
    for part in name.split(".")[1:]:
        target = getattr(target, part)
    return target


def _stmt_define(namespace, name, index, operator, value, has_index):
    special = config.special_namespaces.get(namespace)
    if special is not None:
        if has_index or operator != "=":
            raise NotImplementedError("compound define in special namespace")
        special.set(name, value)
        return
    target = _get_store(namespace).__dict__
    if has_index:
        target, name = target[name], index
    if operator == "+=":
        target[name] += value
    elif operator == "|=":
        target[name] |= value
    else:
        target[name] = value


_defaults = []


def _stmt_default(namespace, name, source, filename, lineno):
    special = config.special_namespaces.get(namespace)
    if special is not None:
        special.set_default(name, eval(source, store.__dict__))
        return
    _defaults.append((namespace, name, source))


def execute_default_statement(start=False):
    for namespace, name, source in _defaults:
        target = _get_store(namespace).__dict__
        already_set = target.setdefault("_defaults_set", set())
        if name not in already_set:
            target[name] = eval(source, store.__dict__)
            already_set.add(name)
        elif start and config.developer:
            raise Exception(namespace + "." + name + " has two default statements")


def _stmt_user(parsed):
    execute, next_node, arguments = parsed
    next_label = None if next_node is None else next_node(arguments)
    if execute is not None:
        execute(arguments)
    if next_label is not None:
        jump(next_label)


def _stmt_post_user(parsed):
    raise NotImplementedError("creator-defined statement post-execution")
