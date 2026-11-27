# SPDX-License-Identifier: MPL-2.0
# Class names and pickle paths follow Ren'Py's renpy/revertable.py (MIT).
"""Rollback-aware containers. CrySirror restores everything reachable from
the stores at a checkpoint, so these classes need no mutation journal; they
keep upstream names, construction and copy/slice result types."""
import random as _random


def _wrap(base, cls, names):
    def make(name):
        method = getattr(base, name)

        def wrapper(self, *args):
            rv = method(self, *args)
            if type(rv) is base:
                rv = cls(rv)
            return rv
        wrapper.__name__ = name
        return wrapper
    for n in names:
        if hasattr(base, n):
            setattr(cls, n, make(n))


class CompressedList(object):
    def __init__(self, old, new):
        self.old = old
        self.new = new

    def decompress(self, new):
        return new

    def __repr__(self):
        return "<CompressedList>"


class RevertableList(list):
    def __init__(self, *args):
        list.__init__(self, *args)

    def copy(self):
        return RevertableList(self)

    def _clean(self):
        return self[:]

    def _compress(self, clean):
        return clean

    def _rollback(self, compressed):
        self[:] = compressed


_wrap(list, RevertableList, ("__add__", "__radd__", "__mul__", "__rmul__", "__getitem__", "__getslice__"))


def revertable_range(*args):
    return RevertableList(range(*args))


def revertable_sorted(*args, **kwargs):
    return RevertableList(sorted(*args, **kwargs))


class RevertableDict(dict):
    def __init__(self, *args, **kwargs):
        dict.__init__(self, *args, **kwargs)

    def copy(self):
        rv = RevertableDict()
        rv.update(self)
        return rv

    def _clean(self):
        return list(self.items())

    def _compress(self, clean):
        return clean

    def _rollback(self, compressed):
        self.clear()
        for k, v in compressed:
            self[k] = v

    def __or__(self, other):
        rv = self.copy()
        rv.update(other)
        return rv


class RevertableDefaultDict(RevertableDict):
    def __init__(self, default_factory=None, *args, **kwargs):
        RevertableDict.__init__(self, *args, **kwargs)
        self.default_factory = default_factory

    def __missing__(self, key):
        if self.default_factory is None:
            raise KeyError(key)
        rv = self[key] = self.default_factory()
        return rv


class RevertableSet(set):
    def __setstate__(self, state):
        if isinstance(state, tuple):
            self.update(state[0].keys())
        else:
            self.update(state)

    def __getstate__(self):
        rv = ({i: True for i in self},)
        return rv

    def __init__(self, *args):
        set.__init__(self, *args)

    def _clean(self):
        return list(self)

    def _compress(self, clean):
        return clean

    def _rollback(self, compressed):
        set.clear(self)
        set.update(self, compressed)


_wrap(set, RevertableSet, ("__and__", "__or__", "__sub__", "__xor__", "__rand__", "__ror__", "__rsub__", "__rxor__",
                           "copy", "difference", "intersection", "symmetric_difference", "union"))


class RevertableObject(object):
    def __new__(cls, *args, **kwargs):
        return object.__new__(cls)

    def __init__(self, *args, **kwargs):
        if (args or kwargs) and type(self).__init__ is RevertableObject.__init__:
            raise TypeError("object() takes no parameters.")

    def _clean(self):
        return self.__dict__.copy()

    def _compress(self, clean):
        return clean

    def _rollback(self, compressed):
        self.__dict__.clear()
        self.__dict__.update(compressed)


class MultiRevertable(object):
    def _rollback_types(self):
        return [t for t in type(self).__mro__ if hasattr(t, "_rollback") and t is not MultiRevertable]

    def _clean(self):
        return self.__dict__.copy()

    def _compress(self, clean):
        return clean

    def _rollback(self, compressed):
        self.__dict__.clear()
        self.__dict__.update(compressed)


def checkpointing(method):
    def do_checkpoint(self, *args, **kwargs):
        import renpy.exports
        renpy.exports.checkpoint()
        return method(self, *args, **kwargs)
    return do_checkpoint


def mutator(method):
    return method


class RollbackRandom(_random.Random):
    def __init__(self):
        super(RollbackRandom, self).__init__()

    def _clean(self):
        return self.getstate()

    def _compress(self, clean):
        return clean

    def _rollback(self, compressed):
        self.setstate(compressed)

    def Random(self, seed=None):
        return DetRandom() if seed is None else _random.Random(seed)


class DetRandom(_random.Random):
    def __init__(self):
        super(DetRandom, self).__init__()
        self.stack = []

    def random(self):
        if self.stack:
            return self.stack.pop()
        return super(DetRandom, self).random()

    def pushback(self, l):
        l = list(l)
        l.reverse()
        self.stack.extend(l)

    def reset(self):
        self.stack = []

    def Random(self, seed=None):
        return _random.Random(seed)
