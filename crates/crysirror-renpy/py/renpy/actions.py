# SPDX-License-Identifier: MPL-2.0
"""Default interaction callbacks for script-defined UI actions in 6.99."""

from renpy.object import Object


class Action(Object):
    alt = None

    def get_sensitive(self):
        return True

    def get_selected(self):
        return False

    def periodic(self, elapsed):
        return None

    def predict(self):
        return None

    def __call__(self):
        raise NotImplementedError("an action must implement __call__")


class BarValue(Object):
    alt = "Bar"

    def replaces(self, other):
        return None

    def periodic(self, elapsed):
        return None

    def get_adjustment(self):
        raise NotImplementedError("a bar value must provide an adjustment")

    def get_style(self):
        return ("bar", "vbar")
