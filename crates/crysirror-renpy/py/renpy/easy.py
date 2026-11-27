# SPDX-License-Identifier: MPL-2.0
"""Native shorthand conversions used by Ren'Py styles."""
from renpy import config
from renpy.displayables import Color, Displayable, Image, ImageReference, Null, Solid

color = Color


def displayable(value, scope=None, dynamic=True):
    if isinstance(value, (Displayable, bool)) or hasattr(value, "_duplicate"):
        return value
    if isinstance(value, str):
        if not value:
            raise ValueError("empty displayable name")
        if "[" in value and dynamic and config.dynamic_images:
            raise NotImplementedError("dynamic image substitutions")
        if value.startswith("#"):
            return Solid(value)
        if "." in value:
            return Image(value)
        return ImageReference(value)
    raise TypeError("not a Ren'Py displayable: " + repr(value))


def displayable_or_none(value, scope=None, dynamic=True):
    return None if value is None else displayable(value, scope, dynamic)


def none_is_null(value):
    return Null() if value is None else displayable(value)
