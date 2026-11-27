# SPDX-License-Identifier: MPL-2.0
# Declarations follow Ren'Py's renpy/object.py (MIT); pickle paths match.
"""Versioned base object and named sentinels."""
import builtins

builtins.NoneType = type(None)


class Object(object):
    __version__ = 0
    nosave = []
    after_setstate = None

    def __getstate__(self):
        rv = vars(self).copy()
        for f in self.nosave:
            rv.pop(f, None)
        rv["__version__"] = self.__version__
        return rv

    def __setstate__(self, new_dict):
        version = new_dict.pop("__version__", 0)
        self.__dict__.update(new_dict)
        if version != self.__version__:
            self.after_upgrade(version)
        if self.after_setstate:
            self.after_setstate()

    def after_upgrade(self, version):
        return


sentinels = {}


class Sentinel(object):
    def __new__(cls, name):
        rv = sentinels.get(name, None)
        if rv is None:
            rv = object.__new__(cls)
            sentinels[name] = rv
        return rv

    def __init__(self, name):
        self.name = name

    def __reduce__(self):
        return (Sentinel, (self.name,))

    def __repr__(self):
        return "<Sentinel {}>".format(self.name)
