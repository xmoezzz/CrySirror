# SPDX-License-Identifier: MPL-2.0
"""Sound uses the same channel registry as music with a different default."""

from renpy import music
from renpy.music import set_mixer, channel_defined, get_all_mixers


def play(filename, channel="sound", **kwargs):
    return music.play(filename, channel=channel, **kwargs)


def stop(channel="sound", **kwargs):
    return music.stop(channel=channel, **kwargs)


def is_playing(channel="sound"):
    return music.is_playing(channel)
