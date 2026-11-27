# SPDX-License-Identifier: MPL-2.0
# Names follow Ren'Py's renpy/compat/pickle.py (MIT).
"""Save pickling at Ren'Py's protocol, reading Python 2 pickles as text."""
import io
import pickle

PROTOCOL = 2
HIGHEST_PROTOCOL = pickle.HIGHEST_PROTOCOL


def load(f):
    return pickle.load(f, encoding="utf-8", errors="surrogateescape")


def loads(s):
    return pickle.loads(s, encoding="utf-8", errors="surrogateescape")


def dump(o, f, highest=False):
    pickle.dump(o, f, pickle.HIGHEST_PROTOCOL if highest else PROTOCOL)


def dumps(o, highest=False):
    return pickle.dumps(o, pickle.HIGHEST_PROTOCOL if highest else PROTOCOL)
