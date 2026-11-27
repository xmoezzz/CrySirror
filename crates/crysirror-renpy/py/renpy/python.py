# SPDX-License-Identifier: MPL-2.0
# Names and pickle paths follow Ren'Py's renpy/python.py (MIT).
"""Stores, embedded-Python evaluation and rollback declarations. Compiling
and evaluating script Python is native (Ren'Py's wrap_node transform and
the per-file Python 2/3 dialect); rollback checkpoints are owned by the
CrySirror executor."""
import _renpy
import renpy.object
from renpy.revertable import (RevertableList, RevertableDict, RevertableSet,
                              RevertableObject, RevertableDefaultDict,
                              MultiRevertable, CompressedList, RollbackRandom,
                              DetRandom, revertable_range, revertable_sorted,
                              mutator, checkpointing)


class StoreDeleted(object):
    def __reduce__(self):
        return "deleted"


deleted = StoreDeleted()


class StoreModule(object):
    def __reduce__(self):
        return (get_store_module, (self.__name__,))

    def __init__(self, d):
        object.__setattr__(self, "__dict__", d)

    def __delattr__(self, key):
        del self.__dict__[key]


def get_store_module(name):
    return _renpy.store_module(name)


class StoreDict(dict):
    def __reduce__(self):
        raise Exception("Cannot pickle a reference to a store dictionary.")

    def __init__(self):
        dict.__init__(self)
        self.old = None
        self.ever_been_changed = set()

    def reset(self):
        self.old = dict(self)
        self.ever_been_changed.clear()

    def begin(self):
        self.old = dict(self)

    def get_changes(self, cycle):
        new = dict(self)
        old = self.old or {}
        rv = {}
        for k in set(new) | set(old):
            if k not in new:
                rv[k] = old[k]
            elif k not in old:
                rv[k] = deleted
            elif new[k] is not old[k]:
                rv[k] = old[k]
        if cycle:
            self.old = new
        return rv, set()


def begin_stores():
    pass


store_dicts = _renpy.store_dicts()
store_modules = _renpy.store_modules()
initialized_store_dicts = set()


def create_store(name):
    return _renpy.create_store(name)


class StoreBackup(object):
    def __init__(self):
        self.store = _renpy.backup_stores()

    def restore(self):
        _renpy.restore_stores(self.store)

    def backup_one(self, name):
        _renpy.backup_one_store(self.store, name)

    def restore_one(self, name):
        _renpy.restore_one_store(self.store, name)


clean_store_backup = None


def make_clean_stores():
    global clean_store_backup
    clean_store_backup = StoreBackup()


def clean_stores():
    if clean_store_backup is not None:
        clean_store_backup.restore()


def clean_store(name):
    if clean_store_backup is not None:
        clean_store_backup.restore_one(name)


def reset_store_changes(name):
    pass


class NoRollback(object):
    pass


class SlottedNoRollback(object):
    __slots__ = []


class AlwaysRollback(RevertableObject):
    def __new__(cls, *args, **kwargs):
        return RevertableObject.__new__(cls)


class Rollback(renpy.object.Object):
    __version__ = 5

    identifier = None
    not_greedy = False
    checkpoint = False
    hard_checkpoint = False

    def __init__(self):
        self.context = None
        self.objects = []
        self.purged = False
        self.random = []
        self.forward = []
        self.stores = {}
        self.delta_ebc = {}
        self.retain_after_load = False
        self.checkpoint = False
        self.hard_checkpoint = False


class RollbackLog(renpy.object.Object):
    __version__ = 5

    nosave = ["old_store", "mutated", "identifier_cache"]
    identifier_cache = None
    force_checkpoint = False

    def __init__(self):
        self.log = []
        self.current = None
        self.mutated = {}
        self.rollback_limit = 0
        self.rollback_is_fixed = False
        self.checkpointing_suspended = False
        self.fixed_rollback_boundary = None
        self.forward = []
        self.old_store = {}
        self.rolled_forward = False
        self.retain_after_load_flag = False
        self.did_interaction = True

    def after_setstate(self):
        self.mutated = {}
        self.rolled_forward = False

    def begin(self, force=False):
        pass

    def complete(self, begin=False):
        pass

    def get_roots(self):
        return _renpy.rollback_roots()

    def purge_unreachable(self, roots, wait=None):
        return False

    def in_rollback(self):
        return _renpy.in_rollback()

    def in_fixed_rollback(self):
        return _renpy.in_fixed_rollback()

    def forward_info(self):
        return _renpy.roll_forward_info()

    def checkpoint(self, data=None, keep_rollback=False, hard=True):
        _renpy.checkpoint(data, hard)

    def suspend_checkpointing(self, flag):
        _renpy.suspend_rollback(flag)

    def block(self, purge=False):
        _renpy.block_rollback()

    def retain_after_load(self):
        _renpy.retain_after_load()

    def fix_rollback(self):
        _renpy.fix_rollback()

    def can_rollback(self):
        return _renpy.rollback_available() > 0

    def load_failed(self):
        raise Exception("Loading the save failed.")

    def rollback(self, checkpoints, force=False, label=None, greedy=True, on_load=False, abnormal=True, current_label=None):
        _renpy.rollback(checkpoints, force, label, greedy, on_load, abnormal, current_label)

    def freeze(self, wait=None):
        return _renpy.freeze_roots()

    def discard_freeze(self):
        pass

    def complete_rollback(self):
        pass


def py_compile(source, mode, filename="<none>", lineno=1, ast_node=False, cache=True, py=None):
    return _renpy.py_compile(source, mode, filename, lineno, py)


def py_compile_exec_bytecode(source, **kwargs):
    return py_compile(source, "exec", **kwargs)


def py_compile_hide_bytecode(source, **kwargs):
    return py_compile(source, "hide", **kwargs)


def py_compile_eval_bytecode(source, **kwargs):
    source = source.strip()
    return py_compile(source, "eval", **kwargs)


def py_exec_bytecode(bytecode, hide=False, globals=None, locals=None, store="store"):
    if globals is None:
        globals = store_dicts[store]
    if hide:
        locals = {}
    exec(bytecode, globals, locals)


def py_exec(source, hide=False, store=None):
    if store is None:
        store = store_dicts["store"]
    else:
        store = store_dicts[store]
    if hide:
        locals = {}
    else:
        locals = store
    exec(py_compile(source, "exec"), store, locals)


def py_eval_bytecode(bytecode, globals=None, locals=None):
    if globals is None:
        globals = store_dicts["store"]
    return eval(bytecode, globals, locals)


def py_eval(code, globals=None, locals=None):
    if isinstance(code, str):
        code = py_compile(code, "eval")
    return py_eval_bytecode(code, globals, locals)


def store_eval(code, globals=None, locals=None):
    if globals is None:
        globals = sys._getframe(1).f_globals
    return py_eval(code, globals, locals)


def raise_at_location(e, loc):
    raise e


class StoreProxy(object):
    def __getattr__(self, k):
        return getattr(renpy.store, k)

    def __setattr__(self, k, v):
        setattr(renpy.store, k, v)

    def __delattr__(self, k):
        delattr(renpy.store, k)


def method_unpickle(obj, name):
    return getattr(obj, name)


def module_pickle(module):
    return module_unpickle, (module.__name__,)


def module_unpickle(name):
    return __import__(name)


def construct_None(*args):
    return None


import sys
import renpy
