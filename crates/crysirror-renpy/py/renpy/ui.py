# SPDX-License-Identifier: MPL-2.0
"""Script UI callbacks and the retained interaction entry point."""
import _renpy
from renpy import Jump, curried_call_in_new_context


def jumps(label):
    return Jump(label)


def callsinnewcontext(label, *args, **kwargs):
    return curried_call_in_new_context(label, *args, **kwargs)


def interact(suppress_overlay=False, suppress_underlay=False, mouse=None,
             type=None, roll_forward=None, pause=None):
    return _renpy.interact(suppress_overlay, suppress_underlay, mouse, type,
                           roll_forward, pause)
