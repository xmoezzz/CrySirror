# SPDX-License-Identifier: MPL-2.0
"""Register script callbacks using the Ren'Py 6.99 argument layout."""
import _renpy


def register(name, parse=None, lint=None, execute=None, predict=None, next=None, scry=None,
             block=False, init=False, translatable=False):
    _renpy.register_statement(name, parse, execute, next, block, init)
