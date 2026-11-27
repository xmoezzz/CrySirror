# SPDX-License-Identifier: MPL-2.0
"""Ren'Py exports under their original module import path."""
from renpy import *
import sys


def fsencode(value):
    if isinstance(value, str):
        return value.encode(sys.getfilesystemencoding() or "utf-8", "replace")
    return value


def fsdecode(value):
    if isinstance(value, bytes):
        return value.decode(sys.getfilesystemencoding() or "utf-8", "replace")
    return value
