# SPDX-License-Identifier: MPL-2.0
"""Script declarations consumed by screen expression analysis."""

constants = set()
pure_functions = set()
not_constants = set()


def const(name):
    if name not in not_constants:
        constants.add(name)


def not_const(name):
    constants.discard(name)
    pure_functions.discard(name)
    not_constants.add(name)


def pure(function):
    name = function if isinstance(function, str) else function.__name__
    if name not in not_constants:
        constants.add(name)
        pure_functions.add(name)
    return function
