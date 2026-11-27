# SPDX-License-Identifier: MPL-2.0
"""Ren'Py channel names and playback policy over the shared CrySirror mixer."""

import _renpy
from renpy import config


class Channel:
    def __init__(self, id, mixer, loop, stop_on_mute, tight, file_prefix,
                 file_suffix, buffer_queue, movie):
        self.id = id
        self.mixer = mixer
        self.loop = loop
        self.loop_set = loop is not None
        self.stop_on_mute = stop_on_mute
        self.tight = tight
        self.file_prefix = file_prefix
        self.file_suffix = file_suffix
        self.buffer_queue = buffer_queue
        self.movie = movie


_channels = {}
_all_channels = []


def register_channel(name, mixer=None, loop=None, stop_on_mute=True, tight=False,
                     file_prefix="", file_suffix="", buffer_queue=True, movie=False):
    if not _renpy.in_init():
        raise Exception("audio channels must be registered during init")
    if len(_all_channels) >= 128:
        raise ValueError("too many audio channels")
    channel = Channel(len(_all_channels), mixer, loop, stop_on_mute, tight,
                      file_prefix, file_suffix, buffer_queue, movie or name == "movie")
    _all_channels.append(channel)
    _channels[name] = channel


def alias_channel(name, newname):
    if not _renpy.in_init():
        raise Exception("audio channels must be aliased during init")
    _channels[newname] = _channels[name]


def set_mixer(channel, mixer, default=False):
    value = _channels[channel]
    if not default or value.mixer is None:
        value.mixer = mixer


def set_music(channel, flag, default=False):
    value = _channels[channel]
    if not default or not value.loop_set:
        value.loop = flag
        value.loop_set = True


def channel_defined(channel):
    return channel in _channels


def get_all_mixers():
    return list(set(value.mixer for value in _all_channels))


def get_playing(channel="music"):
    return _renpy.audio_current(_channels[channel].id)


def is_playing(channel="music"):
    return get_playing(channel) is not None


def play(filenames, channel="music", loop=None, fadeout=None, synchro_start=False,
         fadein=0, tight=None, if_changed=False):
    if _renpy.in_init():
        raise Exception("cannot play during init")
    if filenames is None:
        return
    if synchro_start or tight:
        raise NotImplementedError("synchronized or tight Ren'Py audio")
    if not isinstance(filenames, str):
        if len(filenames) != 1:
            raise NotImplementedError("multi-track Ren'Py playback")
        filenames = filenames[0]
    value = _channels[channel]
    if value.movie:
        raise NotImplementedError("Ren'Py video playback")
    if loop is None:
        loop = value.loop
    if fadeout is None:
        fadeout = config.fade_music
    _renpy.audio_play(value.id, filenames, value.file_prefix + filenames + value.file_suffix,
                      value.mixer, bool(loop), fadeout, fadein, bool(if_changed))


def stop(channel="music", fadeout=None):
    if _renpy.in_init():
        return
    if fadeout is None:
        fadeout = config.fade_music
    _renpy.audio_stop(_channels[channel].id, fadeout)
