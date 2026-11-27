# SPDX-License-Identifier: MPL-2.0
# Declarations follow Ren'Py's renpy/curry.py (MIT); pickle paths match.
import functools


class Curry(object):
    hash = None

    def __init__(self, callable, *args, **kwargs):
        self.callable = callable
        self.args = args
        self.kwargs = kwargs
        self.__doc__ = getattr(self.callable, "__doc__", None)
        self.__name__ = getattr(self.callable, "__name__", None)

    def __call__(self, *args, **kwargs):
        merged = dict(self.kwargs)
        merged.update(kwargs)
        return self.callable(*(self.args + args), **merged)

    def __repr__(self):
        return "<curry %s %r %r>" % (self.callable, self.args, self.kwargs)

    def __eq__(self, other):
        return (isinstance(other, Curry) and self.callable == other.callable and
                self.args == other.args and self.kwargs == other.kwargs)

    def __ne__(self, other):
        return not self == other

    def __hash__(self):
        if self.hash is None:
            h = hash(self.callable) ^ hash(self.args)
            for i in self.kwargs.items():
                h ^= hash(i)
            self.hash = h
        return self.hash


class Partial(functools.partial):

    def __repr__(self):
        return "<partial %s %r %r>" % (self.func, self.args, self.keywords)

    def __eq__(self, other):
        return (isinstance(other, Partial) and self.func == other.func and
                self.args == other.args and self.keywords == other.keywords)

    def __ne__(self, other):
        return not self == other

    def __hash__(self):
        h = hash(self.func) ^ hash(self.args)
        for i in self.keywords.items():
            h ^= hash(i)
        return h


# Curry is used for both helpers; Partial is kept so 8.x pickles load.
def curry(fn):
    return Curry(Curry, fn)


def partial(function, *args, **kwargs):
    return Curry(function, *args, **kwargs)
