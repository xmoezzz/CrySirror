// SPDX-License-Identifier: MPL-2.0
//! Python-source standard library modules, written for this interpreter on
//! top of the native `_`-prefixed modules. They follow the documented
//! behaviour of the CPython modules of the same name.

pub fn is_package(name: &str) -> bool {
    matches!(name, "collections" | "json" | "os" | "importlib" | "encodings" | "xml" | "email" | "urllib" | "concurrent" | "future" | "concurrent.futures")
}

pub fn source(name: &str) -> Option<&'static str> {
    Some(match name {
        "functools" => FUNCTOOLS,
        "collections" => COLLECTIONS,
        "collections.abc" | "_collections_abc" => COLLECTIONS_ABC,
        "copy" => COPY,
        "types" => TYPES,
        "string" => STRING,
        "keyword" => KEYWORD,
        "__future__" => FUTURE,
        "abc" => ABC,
        "contextlib" => CONTEXTLIB,
        "enum" => ENUM,
        "heapq" => HEAPQ,
        "bisect" => BISECT,
        "textwrap" => TEXTWRAP,
        "weakref" => WEAKREF,
        "warnings" => WARNINGS,
        "traceback" => TRACEBACK,
        "re" => RE,
        "sre_constants" | "sre_compile" | "sre_parse" => SRE_COMPAT,
        "json" => JSON,
        "json.decoder" => JSON_DECODER,
        "json.encoder" => "from json import JSONEncoder\n",
        "random" => RANDOM,
        "io" => IO,
        "StringIO" | "cStringIO" => "from io import StringIO\n",
        "os" => OS,
        "os.path" | "posixpath" => OS_PATH,
        "struct" => "from _struct import *\nfrom _struct import Struct, error\n",
        "hashlib" => "from _hashlib import *\n",
        "base64" => BASE64,
        "pickle" | "cPickle" => "from _pickle import *\nfrom _pickle import Pickler, Unpickler, PickleError, PicklingError, UnpicklingError, HIGHEST_PROTOCOL, DEFAULT_PROTOCOL\n",
        "datetime" => DATETIME,
        "colorsys" => COLORSYS,
        "typing" => TYPING,
        "numbers" => NUMBERS,
        "codecs" => CODECS,
        "unicodedata" => UNICODEDATA,
        "locale" => LOCALE,
        "platform" => PLATFORM,
        "fnmatch" => FNMATCH,
        "threading" | "_thread" | "thread" | "dummy_threading" => THREADING,
        "inspect" => INSPECT,
        "linecache" => "def getline(filename, lineno, module_globals=None):\n    return ''\ndef clearcache():\n    pass\ndef checkcache(filename=None):\n    pass\n",
        "pprint" => PPRINT,
        "reprlib" | "repr" => REPRLIB,
        "fractions" => FRACTIONS,
        "dataclasses" => DATACLASSES,
        "statistics" => STATISTICS,
        "operator" | "itertools" => return None,
        "builtins" | "__builtin__" => return None,
        "future" => "",
        "future.utils" => FUTURE_UTILS,
        "six" => SIX,
        "importlib" => "import sys\ndef import_module(name, package=None):\n    if name.startswith('.'):\n        name = package + name\n    __import__(name)\n    return sys.modules[name]\ndef reload(module):\n    return module\ndef invalidate_caches():\n    pass\n",
        "queue" | "Queue" => QUEUE,
        "atexit" => ATEXIT,
        "shutil" => "import os\ndef copyfile(src, dst):\n    with open(src, 'rb') as a:\n        d = a.read()\n    with open(dst, 'wb') as b:\n        b.write(d)\n    return dst\ncopy = copyfile\ncopy2 = copyfile\ndef move(src, dst):\n    os.rename(src, dst)\n    return dst\ndef rmtree(path, ignore_errors=False):\n    pass\n",
        "glob" => "import os, fnmatch\ndef glob(pattern):\n    d, p = os.path.split(pattern)\n    try:\n        names = os.listdir(d or '.')\n    except OSError:\n        return []\n    return [os.path.join(d, n) if d else n for n in names if fnmatch.fnmatch(n, p)]\niglob = glob\n",
        "calendar" => CALENDAR,
        "uuid" => UUID,
        "array" => ARRAY,
        _ => return None,
    })
}

const FUNCTOOLS: &str = r##"
from _functools import reduce, partial, cmp_to_key

WRAPPER_ASSIGNMENTS = ('__module__', '__name__', '__qualname__', '__doc__')
WRAPPER_UPDATES = ('__dict__',)

def update_wrapper(wrapper, wrapped, assigned=WRAPPER_ASSIGNMENTS, updated=WRAPPER_UPDATES):
    for attr in assigned:
        try:
            value = getattr(wrapped, attr)
        except AttributeError:
            pass
        else:
            try:
                setattr(wrapper, attr, value)
            except (AttributeError, TypeError):
                pass
    for attr in updated:
        try:
            getattr(wrapper, attr).update(getattr(wrapped, attr, {}))
        except (AttributeError, TypeError):
            pass
    try:
        wrapper.__wrapped__ = wrapped
    except (AttributeError, TypeError):
        pass
    return wrapper

def wraps(wrapped, assigned=WRAPPER_ASSIGNMENTS, updated=WRAPPER_UPDATES):
    return partial(update_wrapper, wrapped=wrapped, assigned=assigned, updated=updated)

def total_ordering(cls):
    roots = {op for op in ('__lt__', '__le__', '__gt__', '__ge__') if getattr(cls, op, None) is not getattr(object, op, None)}
    if not roots:
        raise ValueError('must define at least one ordering operation: < > <= >=')
    root = max(roots)
    def lt_from_le(self, other):
        r = type(self).__le__(self, other)
        return r if r is NotImplemented else (r and self != other)
    def gt_from_le(self, other):
        r = type(self).__le__(self, other)
        return r if r is NotImplemented else not r
    def ge_from_le(self, other):
        r = type(self).__le__(self, other)
        return r if r is NotImplemented else (not r or self == other)
    def le_from_lt(self, other):
        r = type(self).__lt__(self, other)
        return r if r is NotImplemented else (r or self == other)
    def gt_from_lt(self, other):
        r = type(self).__lt__(self, other)
        return r if r is NotImplemented else (not r and self != other)
    def ge_from_lt(self, other):
        r = type(self).__lt__(self, other)
        return r if r is NotImplemented else not r
    def lt_from_gt(self, other):
        r = type(self).__gt__(self, other)
        return r if r is NotImplemented else (not r and self != other)
    def le_from_gt(self, other):
        r = type(self).__gt__(self, other)
        return r if r is NotImplemented else not r
    def ge_from_gt(self, other):
        r = type(self).__gt__(self, other)
        return r if r is NotImplemented else (r or self == other)
    def lt_from_ge(self, other):
        r = type(self).__ge__(self, other)
        return r if r is NotImplemented else not r
    def le_from_ge(self, other):
        r = type(self).__ge__(self, other)
        return r if r is NotImplemented else (not r or self == other)
    def gt_from_ge(self, other):
        r = type(self).__ge__(self, other)
        return r if r is NotImplemented else (r and self != other)
    convert = {
        '__lt__': [('__gt__', gt_from_lt), ('__le__', le_from_lt), ('__ge__', ge_from_lt)],
        '__le__': [('__ge__', ge_from_le), ('__lt__', lt_from_le), ('__gt__', gt_from_le)],
        '__gt__': [('__lt__', lt_from_gt), ('__ge__', ge_from_gt), ('__le__', le_from_gt)],
        '__ge__': [('__le__', le_from_ge), ('__gt__', gt_from_ge), ('__lt__', lt_from_ge)],
    }
    for opname, opfunc in convert[root]:
        if opname not in roots:
            opfunc.__name__ = opname
            setattr(cls, opname, opfunc)
    return cls

class _HashedSeq(list):
    def __init__(self, tup):
        self[:] = tup
        self.hashvalue = hash(tup)
    def __hash__(self):
        return self.hashvalue

def _make_key(args, kwds, typed):
    key = args
    if kwds:
        key += (object,)
        for item in kwds.items():
            key += item
    if typed:
        key += tuple(type(v) for v in args)
    if len(key) == 1 and type(key[0]) in (int, str):
        return key[0]
    return _HashedSeq(key)

def lru_cache(maxsize=128, typed=False):
    if callable(maxsize) and not isinstance(maxsize, int):
        return lru_cache(128, False)(maxsize)
    def decorating_function(user_function):
        cache = {}
        order = []
        stats = [0, 0]
        def wrapper(*args, **kwds):
            key = _make_key(args, kwds, typed)
            if key in cache:
                stats[0] += 1
                return cache[key]
            stats[1] += 1
            result = user_function(*args, **kwds)
            cache[key] = result
            if maxsize is not None:
                order.append(key)
                if len(order) > maxsize:
                    old = order.pop(0)
                    cache.pop(old, None)
            return result
        def cache_clear():
            cache.clear()
            del order[:]
        def cache_info():
            return (stats[0], stats[1], maxsize, len(cache))
        wrapper.cache_clear = cache_clear
        wrapper.cache_info = cache_info
        return update_wrapper(wrapper, user_function)
    return decorating_function

def cache(user_function):
    return lru_cache(maxsize=None)(user_function)

class cached_property(object):
    def __init__(self, func):
        self.func = func
        self.attrname = None
        self.__doc__ = func.__doc__
    def __set_name__(self, owner, name):
        self.attrname = name
    def __get__(self, instance, owner=None):
        if instance is None:
            return self
        name = self.attrname or self.func.__name__
        val = self.func(instance)
        instance.__dict__[name] = val
        return val

def singledispatch(func):
    registry = {object: func}
    def dispatch(cls):
        for c in cls.__mro__:
            if c in registry:
                return registry[c]
        return func
    def register(cls, f=None):
        if f is None:
            if isinstance(cls, type):
                return lambda f: register(cls, f)
            f = cls
            cls = list(f.__annotations__.values())[0]
        registry[cls] = f
        return f
    def wrapper(*args, **kw):
        return dispatch(args[0].__class__)(*args, **kw)
    wrapper.register = register
    wrapper.dispatch = dispatch
    wrapper.registry = registry
    update_wrapper(wrapper, func)
    return wrapper

class partialmethod(object):
    def __init__(self, func, *args, **keywords):
        self.func = func
        self.args = args
        self.keywords = keywords
    def __get__(self, obj, cls=None):
        if obj is None:
            return self
        return partial(self.func, obj, *self.args, **self.keywords)
"##;

const COLLECTIONS: &str = r##"
from _collections import OrderedDict, defaultdict, deque
import _collections_abc
from _collections_abc import *
import sys as _sys
from keyword import iskeyword as _iskeyword
from operator import itemgetter as _itemgetter

def namedtuple(typename, field_names, rename=False, defaults=None, module=None, verbose=False):
    if isinstance(field_names, str):
        field_names = field_names.replace(',', ' ').split()
    field_names = list(map(str, field_names))
    if rename:
        seen = set()
        for index, name in enumerate(field_names):
            if (not name.isidentifier() or _iskeyword(name) or name.startswith('_') or name in seen):
                field_names[index] = '_%d' % index
            seen.add(name)
    for name in [typename] + field_names:
        if not name.isidentifier():
            raise ValueError('Type names and field names must be valid identifiers: %r' % name)
        if _iskeyword(name):
            raise ValueError('Type names and field names cannot be a keyword: %r' % name)
    seen = set()
    for name in field_names:
        if name.startswith('_') and not rename:
            raise ValueError('Field names cannot start with an underscore: %r' % name)
        if name in seen:
            raise ValueError('Encountered duplicate field name: %r' % name)
        seen.add(name)
    field_names = tuple(field_names)
    num_fields = len(field_names)
    field_defaults = {}
    if defaults is not None:
        defaults = tuple(defaults)
        if len(defaults) > num_fields:
            raise TypeError('Got more default values than field names')
        field_defaults = dict(reversed(list(zip(reversed(field_names), reversed(defaults)))))

    def __new__(_cls, *args, **kwargs):
        if len(args) > num_fields:
            raise TypeError('%s() takes %d positional arguments but %d were given' % (typename, num_fields, len(args)))
        values = list(args)
        for name in field_names[len(args):]:
            if name in kwargs:
                values.append(kwargs.pop(name))
            elif name in field_defaults:
                values.append(field_defaults[name])
            else:
                raise TypeError("%s() missing required argument: '%s'" % (typename, name))
        if kwargs:
            raise TypeError('%s() got an unexpected keyword argument %r' % (typename, list(kwargs)[0]))
        return tuple.__new__(_cls, values)

    def _make(cls, iterable):
        result = tuple.__new__(cls, iterable)
        if len(result) != num_fields:
            raise TypeError('Expected %d arguments, got %d' % (num_fields, len(result)))
        return result

    def _replace(self, **kwds):
        result = self._make([kwds.pop(n, v) for n, v in zip(field_names, self)])
        if kwds:
            raise ValueError('Got unexpected field names: %r' % list(kwds))
        return result

    def __repr__(self):
        return self.__class__.__name__ + '(' + ', '.join('%s=%r' % (n, v) for n, v in zip(field_names, self)) + ')'

    def _asdict(self):
        return dict(zip(self._fields, self))

    def __getnewargs__(self):
        return tuple(self)

    ns = {
        '__doc__': '%s(%s)' % (typename, ', '.join(field_names)),
        '__slots__': (),
        '_fields': field_names,
        '_field_defaults': field_defaults,
        '__new__': __new__,
        '_make': classmethod(_make),
        '_replace': _replace,
        '__repr__': __repr__,
        '_asdict': _asdict,
        '__getnewargs__': __getnewargs__,
        '__match_args__': field_names,
    }
    for index, name in enumerate(field_names):
        ns[name] = property(_itemgetter(index), doc='Alias for field number %d' % index)
    result = type(typename, (tuple,), ns)
    if module is None:
        try:
            module = _sys._getframe(1).f_globals.get('__name__', '__main__')
        except (AttributeError, ValueError):
            module = '__main__'
    result.__module__ = module
    return result

class Counter(dict):
    def __init__(self, iterable=None, **kwds):
        dict.__init__(self)
        self.update(iterable, **kwds)
    def __missing__(self, key):
        return 0
    def total(self):
        return sum(self.values())
    def most_common(self, n=None):
        items = sorted(self.items(), key=_itemgetter(1), reverse=True)
        return items if n is None else items[:n]
    def elements(self):
        for elem, count in self.items():
            for _ in range(count):
                yield elem
    def update(self, iterable=None, **kwds):
        if iterable is not None:
            if hasattr(iterable, 'keys'):
                for elem in iterable.keys():
                    self[elem] = self.get(elem, 0) + iterable[elem]
            else:
                for elem in iterable:
                    self[elem] = self.get(elem, 0) + 1
        if kwds:
            self.update(kwds)
    def subtract(self, iterable=None, **kwds):
        if iterable is not None:
            if hasattr(iterable, 'keys'):
                for elem in iterable.keys():
                    self[elem] = self.get(elem, 0) - iterable[elem]
            else:
                for elem in iterable:
                    self[elem] = self.get(elem, 0) - 1
        if kwds:
            self.subtract(kwds)
    def copy(self):
        return self.__class__(self)
    def __reduce__(self):
        return self.__class__, (dict(self),)
    def __delitem__(self, elem):
        if elem in self:
            dict.__delitem__(self, elem)
    def __repr__(self):
        if not self:
            return '%s()' % self.__class__.__name__
        return '%s({%s})' % (self.__class__.__name__, ', '.join('%r: %r' % kv for kv in self.most_common()))
    def __add__(self, other):
        result = Counter()
        for elem, count in self.items():
            newcount = count + other[elem]
            if newcount > 0:
                result[elem] = newcount
        for elem, count in other.items():
            if elem not in self and count > 0:
                result[elem] = count
        return result
    def __sub__(self, other):
        result = Counter()
        for elem, count in self.items():
            newcount = count - other[elem]
            if newcount > 0:
                result[elem] = newcount
        for elem, count in other.items():
            if elem not in self and count < 0:
                result[elem] = 0 - count
        return result
    def __or__(self, other):
        result = Counter()
        for elem, count in self.items():
            other_count = other[elem]
            newcount = other_count if count < other_count else count
            if newcount > 0:
                result[elem] = newcount
        for elem, count in other.items():
            if elem not in self and count > 0:
                result[elem] = count
        return result
    def __and__(self, other):
        result = Counter()
        for elem, count in self.items():
            other_count = other[elem]
            newcount = count if count < other_count else other_count
            if newcount > 0:
                result[elem] = newcount
        return result

class ChainMap(MutableMapping):
    def __init__(self, *maps):
        self.maps = list(maps) or [{}]
    def __missing__(self, key):
        raise KeyError(key)
    def __getitem__(self, key):
        for mapping in self.maps:
            try:
                return mapping[key]
            except KeyError:
                pass
        return self.__missing__(key)
    def get(self, key, default=None):
        return self[key] if key in self else default
    def __len__(self):
        return len(set().union(*self.maps))
    def __iter__(self):
        d = {}
        for mapping in reversed(self.maps):
            d.update(dict.fromkeys(mapping))
        return iter(d)
    def __contains__(self, key):
        return any(key in m for m in self.maps)
    def __setitem__(self, key, value):
        self.maps[0][key] = value
    def __delitem__(self, key):
        del self.maps[0][key]
    def new_child(self, m=None):
        return self.__class__(m or {}, *self.maps)
    @property
    def parents(self):
        return self.__class__(*self.maps[1:])

class UserDict(MutableMapping):
    def __init__(self, dict=None, **kwargs):
        self.data = {}
        if dict is not None:
            self.update(dict)
        if kwargs:
            self.update(kwargs)
    def __len__(self):
        return len(self.data)
    def __getitem__(self, key):
        if key in self.data:
            return self.data[key]
        if hasattr(self.__class__, '__missing__'):
            return self.__class__.__missing__(self, key)
        raise KeyError(key)
    def __setitem__(self, key, item):
        self.data[key] = item
    def __delitem__(self, key):
        del self.data[key]
    def __iter__(self):
        return iter(self.data)
    def __contains__(self, key):
        return key in self.data
    def __repr__(self):
        return repr(self.data)
    def copy(self):
        return self.__class__(self.data)

class UserList(MutableSequence):
    def __init__(self, initlist=None):
        self.data = []
        if initlist is not None:
            self.data[:] = list(initlist)
    def __repr__(self):
        return repr(self.data)
    def __getitem__(self, i):
        return self.data[i]
    def __setitem__(self, i, item):
        self.data[i] = item
    def __delitem__(self, i):
        del self.data[i]
    def __len__(self):
        return len(self.data)
    def insert(self, i, item):
        self.data.insert(i, item)
    def __eq__(self, other):
        return self.data == (other.data if isinstance(other, UserList) else other)

class UserString(Sequence):
    def __init__(self, seq):
        self.data = str(seq)
    def __str__(self):
        return self.data
    def __repr__(self):
        return repr(self.data)
    def __len__(self):
        return len(self.data)
    def __getitem__(self, index):
        return self.__class__(self.data[index])
    def __eq__(self, other):
        return self.data == (other.data if isinstance(other, UserString) else other)
    def __hash__(self):
        return hash(self.data)
"##;

const COLLECTIONS_ABC: &str = r##"
from abc import ABCMeta, abstractmethod

def _check_methods(C, *methods):
    mro = C.__mro__
    for method in methods:
        for B in mro:
            if method in B.__dict__:
                if B.__dict__[method] is None:
                    return NotImplemented
                break
        else:
            return NotImplemented
    return True

class Hashable(metaclass=ABCMeta):
    __slots__ = ()
    @abstractmethod
    def __hash__(self):
        return 0

class Iterable(metaclass=ABCMeta):
    __slots__ = ()
    @abstractmethod
    def __iter__(self):
        while False:
            yield None

class Iterator(Iterable):
    __slots__ = ()
    @abstractmethod
    def __next__(self):
        raise StopIteration
    def __iter__(self):
        return self

class Reversible(Iterable):
    __slots__ = ()

class Generator(Iterator):
    __slots__ = ()

class Sized(metaclass=ABCMeta):
    __slots__ = ()
    @abstractmethod
    def __len__(self):
        return 0

class Container(metaclass=ABCMeta):
    __slots__ = ()
    @abstractmethod
    def __contains__(self, x):
        return False

class Collection(Sized, Iterable, Container):
    __slots__ = ()

class Callable(metaclass=ABCMeta):
    __slots__ = ()
    @abstractmethod
    def __call__(self, *args, **kwds):
        return False

class Set(Collection):
    __slots__ = ()
    def __le__(self, other):
        if len(self) > len(other):
            return False
        for elem in self:
            if elem not in other:
                return False
        return True
    def __lt__(self, other):
        return len(self) < len(other) and self.__le__(other)
    def __ge__(self, other):
        return len(self) >= len(other) and all(e in self for e in other)
    def __gt__(self, other):
        return len(self) > len(other) and self.__ge__(other)
    def __eq__(self, other):
        return len(self) == len(other) and self.__le__(other)
    def __and__(self, other):
        return set(v for v in other if v in self)
    def __or__(self, other):
        return set(list(self) + list(other))
    def __sub__(self, other):
        return set(v for v in self if v not in other)
    def isdisjoint(self, other):
        for value in other:
            if value in self:
                return False
        return True

class MutableSet(Set):
    __slots__ = ()
    def remove(self, value):
        if value not in self:
            raise KeyError(value)
        self.discard(value)
    def pop(self):
        it = iter(self)
        try:
            value = next(it)
        except StopIteration:
            raise KeyError
        self.discard(value)
        return value
    def clear(self):
        try:
            while True:
                self.pop()
        except KeyError:
            pass
    def __ior__(self, it):
        for value in it:
            self.add(value)
        return self
    def __isub__(self, it):
        for value in it:
            self.discard(value)
        return self

class Mapping(Collection):
    __slots__ = ()
    def get(self, key, default=None):
        try:
            return self[key]
        except KeyError:
            return default
    def __contains__(self, key):
        try:
            self[key]
        except KeyError:
            return False
        else:
            return True
    def keys(self):
        return list(self)
    def items(self):
        return [(key, self[key]) for key in self]
    def values(self):
        return [self[key] for key in self]
    def __eq__(self, other):
        if not isinstance(other, Mapping):
            return NotImplemented
        return dict(self.items()) == dict(other.items())

class MutableMapping(Mapping):
    __slots__ = ()
    __marker = object()
    def pop(self, key, default=__marker):
        try:
            value = self[key]
        except KeyError:
            if default is self.__marker:
                raise
            return default
        else:
            del self[key]
            return value
    def popitem(self):
        try:
            key = next(iter(self))
        except StopIteration:
            raise KeyError
        value = self[key]
        del self[key]
        return key, value
    def clear(self):
        try:
            while True:
                self.popitem()
        except KeyError:
            pass
    def update(self, other=(), **kwds):
        if isinstance(other, Mapping):
            for key in other:
                self[key] = other[key]
        elif hasattr(other, "keys"):
            for key in other.keys():
                self[key] = other[key]
        else:
            for key, value in other:
                self[key] = value
        for key, value in kwds.items():
            self[key] = value
    def setdefault(self, key, default=None):
        try:
            return self[key]
        except KeyError:
            self[key] = default
        return default

class Sequence(Reversible, Collection):
    __slots__ = ()
    def __iter__(self):
        i = 0
        try:
            while True:
                v = self[i]
                yield v
                i += 1
        except IndexError:
            return
    def __contains__(self, value):
        for v in self:
            if v is value or v == value:
                return True
        return False
    def __reversed__(self):
        for i in reversed(range(len(self))):
            yield self[i]
    def index(self, value, start=0, stop=None):
        i = start
        while stop is None or i < stop:
            try:
                v = self[i]
            except IndexError:
                break
            if v is value or v == value:
                return i
            i += 1
        raise ValueError
    def count(self, value):
        return sum(1 for v in self if v is value or v == value)

class MutableSequence(Sequence):
    __slots__ = ()
    def append(self, value):
        self.insert(len(self), value)
    def clear(self):
        try:
            while True:
                self.pop()
        except IndexError:
            pass
    def reverse(self):
        n = len(self)
        for i in range(n // 2):
            self[i], self[n - i - 1] = self[n - i - 1], self[i]
    def extend(self, values):
        for v in list(values):
            self.append(v)
    def pop(self, index=-1):
        v = self[index]
        del self[index]
        return v
    def remove(self, value):
        del self[self.index(value)]
    def __iadd__(self, values):
        self.extend(values)
        return self

class ByteString(Sequence):
    __slots__ = ()

class MappingView(Sized):
    __slots__ = ()
class KeysView(MappingView, Set):
    __slots__ = ()
class ItemsView(MappingView, Set):
    __slots__ = ()
class ValuesView(MappingView, Collection):
    __slots__ = ()
class Awaitable(metaclass=ABCMeta):
    __slots__ = ()
class Coroutine(Awaitable):
    __slots__ = ()
class AsyncIterable(metaclass=ABCMeta):
    __slots__ = ()
class AsyncIterator(AsyncIterable):
    __slots__ = ()

for _t in (list, tuple, str, range, bytes):
    Sequence.register(_t)
for _t in (list, bytearray):
    MutableSequence.register(_t)
Mapping.register(dict)
MutableMapping.register(dict)
Set.register(frozenset)
MutableSet.register(set)
Set.register(set)
for _t in (list, tuple, str, dict, set, frozenset, range, bytes, bytearray):
    Iterable.register(_t)
    Sized.register(_t)
    Container.register(_t)
    Collection.register(_t)
for _t in (int, float, complex, str, bytes, tuple, frozenset, type(None), type):
    Hashable.register(_t)
"##;

const ABC: &str = r##"
def abstractmethod(funcobj):
    try:
        funcobj.__isabstractmethod__ = True
    except AttributeError:
        pass
    return funcobj

class abstractclassmethod(classmethod):
    __isabstractmethod__ = True

class abstractstaticmethod(staticmethod):
    __isabstractmethod__ = True

class abstractproperty(property):
    __isabstractmethod__ = True

class ABCMeta(type):
    def __new__(mcls, name, bases, namespace, **kwargs):
        cls = super().__new__(mcls, name, bases, namespace, **kwargs)
        abstracts = {name for name, value in namespace.items() if getattr(value, "__isabstractmethod__", False)}
        for base in bases:
            for name in getattr(base, "__abstractmethods__", set()):
                value = getattr(cls, name, None)
                if getattr(value, "__isabstractmethod__", False):
                    abstracts.add(name)
        cls.__abstractmethods__ = frozenset(abstracts)
        cls._abc_registry = []
        return cls
    def register(cls, subclass):
        cls._abc_registry.append(subclass)
        return subclass
    def __instancecheck__(cls, instance):
        return cls.__subclasscheck__(type(instance))
    def __subclasscheck__(cls, subclass):
        if cls in getattr(subclass, '__mro__', ()):
            return True
        for r in cls._abc_registry:
            if r is subclass or issubclass(subclass, r):
                return True
        for sub in cls.__dict__.get('_abc_subclasses', ()):
            if issubclass(subclass, sub):
                return True
        hook = cls.__dict__.get('__subclasshook__')
        if hook is not None:
            r = hook.__get__(None, cls)(subclass) if hasattr(hook, '__get__') else NotImplemented
            if r is True:
                return True
        return False

class ABC(metaclass=ABCMeta):
    __slots__ = ()

def get_cache_token():
    return 0
"##;

const COPY: &str = r##"
import copyreg as _copyreg

class Error(Exception):
    pass
error = Error

_atomic = (type(None), int, float, bool, complex, str, bytes, type, range, type(len), type(Ellipsis), type(NotImplemented), frozenset)

def copy(x):
    cls = type(x)
    if cls in _atomic:
        return x
    if cls in (list, dict, set, bytearray):
        return cls.copy(x)
    if cls is tuple:
        return x
    copier = getattr(cls, "__copy__", None)
    if copier is not None:
        return copier(x)
    reductor = _copyreg.dispatch_table.get(cls)
    if reductor is not None:
        rv = reductor(x)
    else:
        reductor = getattr(x, "__reduce_ex__", None)
        if reductor is not None:
            rv = reductor(4)
        else:
            reductor = getattr(x, "__reduce__", None)
            if reductor:
                rv = reductor()
            else:
                raise Error("un(shallow)copyable object of type %s" % cls)
    if isinstance(rv, str):
        return x
    return _reconstruct(x, None, *rv)

def deepcopy(x, memo=None, _nil=[]):
    if memo is None:
        memo = {}
    d = id(x)
    y = memo.get(d, _nil)
    if y is not _nil:
        return y
    cls = type(x)
    if cls in _atomic or isinstance(x, type):
        y = x
    elif cls is list:
        y = []
        memo[d] = y
        append = y.append
        for a in x:
            append(deepcopy(a, memo))
    elif cls is tuple:
        y = [deepcopy(a, memo) for a in x]
        try:
            return memo[d]
        except KeyError:
            pass
        for k, j in zip(x, y):
            if k is not j:
                y = tuple(y)
                break
        else:
            y = x
    elif cls is dict:
        y = {}
        memo[d] = y
        for key, value in x.items():
            y[deepcopy(key, memo)] = deepcopy(value, memo)
    elif cls is set:
        y = set()
        memo[d] = y
        for v in x:
            y.add(deepcopy(v, memo))
    elif hasattr(cls, '__mro__') and type(len) is cls:
        y = x
    else:
        copier = getattr(x, "__deepcopy__", None)
        if copier is not None and not isinstance(x, type):
            y = copier(memo)
        else:
            reductor = _copyreg.dispatch_table.get(cls)
            if reductor:
                rv = reductor(x)
            else:
                reductor = getattr(x, "__reduce_ex__", None)
                if reductor is not None:
                    rv = reductor(4)
                else:
                    reductor = getattr(x, "__reduce__", None)
                    if reductor:
                        rv = reductor()
                    else:
                        raise Error("un(deep)copyable object of type %s" % cls)
            if isinstance(rv, str):
                y = x
            else:
                y = _reconstruct(x, memo, *rv)
    if y is not x:
        memo[d] = y
        _keep_alive(x, memo)
    return y

def _keep_alive(x, memo):
    try:
        memo[id(memo)].append(x)
    except KeyError:
        memo[id(memo)] = [x]

def _reconstruct(x, memo, func, args, state=None, listiter=None, dictiter=None, *, deepcopy=deepcopy):
    deep = memo is not None
    if deep and args:
        args = tuple(deepcopy(arg, memo) for arg in args)
    y = func(*args)
    if deep:
        memo[id(x)] = y
    if state is not None:
        if deep:
            state = deepcopy(state, memo)
        if hasattr(y, '__setstate__'):
            y.__setstate__(state)
        else:
            if isinstance(state, tuple) and len(state) == 2:
                state, slotstate = state
            else:
                slotstate = None
            if state is not None:
                y.__dict__.update(state)
            if slotstate is not None:
                for key, value in slotstate.items():
                    setattr(y, key, value)
    if listiter is not None:
        if deep:
            for item in listiter:
                item = deepcopy(item, memo)
                y.append(item)
        else:
            for item in listiter:
                y.append(item)
    if dictiter is not None:
        if deep:
            for key, value in dictiter:
                key = deepcopy(key, memo)
                value = deepcopy(value, memo)
                y[key] = value
        else:
            for key, value in dictiter:
                y[key] = value
    return y
"##;

const TYPES: &str = r##"
import sys
class UnionType:
    """`X | Y` of classes (PEP 604)."""
    __slots__ = ('__args__',)
    @staticmethod
    def _make(a, b):
        args = []
        for x in (a, b):
            for y in (x.__args__ if isinstance(x, UnionType) else (x,)):
                y = type(None) if y is None else y
                if y not in args:
                    args.append(y)
        rv = UnionType.__new__(UnionType)
        rv.__args__ = tuple(args)
        return rv
    def __or__(self, other):
        return UnionType._make(self, other)
    def __ror__(self, other):
        return UnionType._make(other, self)
    def __instancecheck__(self, obj):
        return isinstance(obj, self.__args__)
    def __subclasscheck__(self, cls):
        return issubclass(cls, self.__args__)
    def __eq__(self, other):
        return isinstance(other, UnionType) and set(self.__args__) == set(other.__args__)
    def __hash__(self):
        return hash(frozenset(self.__args__))
    def __repr__(self):
        return ' | '.join('None' if a is type(None) else getattr(a, '__qualname__', repr(a)) if getattr(a, '__module__', None) == 'builtins' else repr(a) if not isinstance(a, type) else a.__module__ + '.' + a.__qualname__ for a in self.__args__)
def _f(): pass
FunctionType = type(_f)
LambdaType = type(lambda: None)
CodeType = type(_f.__code__)
MappingProxyType = type(type.__dict__)
SimpleNamespace = None
def _g():
    yield 1
GeneratorType = type(_g())
class _C:
    def _m(self): pass
MethodType = type(_C()._m)
BuiltinFunctionType = type(len)
BuiltinMethodType = type([].append)
ModuleType = type(sys)
NoneType = type(None)
EllipsisType = type(Ellipsis)
NotImplementedType = type(NotImplemented)
TracebackType = type(None)
FrameType = type(sys._getframe())
GetSetDescriptorType = type(None)
MemberDescriptorType = type(None)
WrapperDescriptorType = type(object.__init__)
MethodWrapperType = type(object().__str__)
MethodDescriptorType = type(str.join)
ClassMethodDescriptorType = type(dict.__dict__['fromkeys'])
StringType = str
UnicodeType = str
StringTypes = (str,)
IntType = int
LongType = int
FloatType = float
BooleanType = bool
ListType = list
TupleType = tuple
DictType = dict
DictionaryType = dict
ObjectType = object
TypeType = type
ClassType = type
InstanceType = object
FileType = object
SliceType = slice
ComplexType = complex
UnboundMethodType = FunctionType
XRangeType = range
del _f, _g, _C

class SimpleNamespace:
    def __init__(self, **kwargs):
        self.__dict__.update(kwargs)
    def __repr__(self):
        items = ("{}={!r}".format(k, self.__dict__[k]) for k in sorted(self.__dict__))
        return "{}({})".format(type(self).__name__, ", ".join(items))
    def __eq__(self, other):
        if isinstance(self, SimpleNamespace) and isinstance(other, SimpleNamespace):
            return self.__dict__ == other.__dict__
        return NotImplemented

def new_class(name, bases=(), kwds=None, exec_body=None):
    ns = {}
    if exec_body is not None:
        exec_body(ns)
    meta = (kwds or {}).get('metaclass', type)
    return meta(name, tuple(bases), ns)

class DynamicClassAttribute(property):
    pass

def coroutine(func):
    return func
"##;

const STRING: &str = r##"
whitespace = ' \t\n\r\v\f'
ascii_lowercase = 'abcdefghijklmnopqrstuvwxyz'
ascii_uppercase = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'
ascii_letters = ascii_lowercase + ascii_uppercase
lowercase = ascii_lowercase
uppercase = ascii_uppercase
letters = ascii_letters
digits = '0123456789'
hexdigits = digits + 'abcdef' + 'ABCDEF'
octdigits = '01234567'
punctuation = r"""!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~"""
printable = digits + ascii_letters + punctuation + whitespace

def capwords(s, sep=None):
    return (sep or ' ').join(x.capitalize() for x in s.split(sep))

def join(words, sep=' '):
    return sep.join(words)

def split(s, sep=None, maxsplit=-1):
    return s.split(sep, maxsplit)

def lower(s):
    return s.lower()

def upper(s):
    return s.upper()

def strip(s, chars=None):
    return s.strip(chars)

def replace(s, old, new, maxreplace=-1):
    return s.replace(old, new, maxreplace)

import re as _re

class Template:
    delimiter = '$'
    idpattern = r'(?a:[_a-z][_a-z0-9]*)'
    braceidpattern = None
    flags = _re.IGNORECASE
    def __init__(self, template):
        self.template = template
        delim = _re.escape(self.delimiter)
        bid = self.braceidpattern or self.idpattern
        self.pattern = _re.compile(r'%s(?:(?P<escaped>%s)|(?P<named>%s)|{(?P<braced>%s)}|(?P<invalid>))' % (delim, delim, self.idpattern, bid), self.flags | _re.VERBOSE if False else self.flags)
    def substitute(self, mapping={}, **kws):
        if kws:
            m = dict(mapping)
            m.update(kws)
            mapping = m
        def convert(mo):
            named = mo.group('named') or mo.group('braced')
            if named is not None:
                return str(mapping[named])
            if mo.group('escaped') is not None:
                return self.delimiter
            raise ValueError('Invalid placeholder in string')
        return self.pattern.sub(convert, self.template)
    def safe_substitute(self, mapping={}, **kws):
        if kws:
            m = dict(mapping)
            m.update(kws)
            mapping = m
        def convert(mo):
            named = mo.group('named') or mo.group('braced')
            if named is not None:
                try:
                    return str(mapping[named])
                except KeyError:
                    return mo.group()
            if mo.group('escaped') is not None:
                return self.delimiter
            return mo.group()
        return self.pattern.sub(convert, self.template)

class Formatter:
    def format(self, format_string, *args, **kwargs):
        return self.vformat(format_string, args, kwargs)
    def vformat(self, format_string, args, kwargs):
        used_args = set()
        result, _ = self._vformat(format_string, args, kwargs, used_args, 2)
        self.check_unused_args(used_args, args, kwargs)
        return result
    def _vformat(self, format_string, args, kwargs, used_args, recursion_depth, auto_arg_index=0):
        if recursion_depth < 0:
            raise ValueError('Max string recursion exceeded')
        result = []
        for literal_text, field_name, format_spec, conversion in self.parse(format_string):
            if literal_text:
                result.append(literal_text)
            if field_name is not None:
                if field_name == '':
                    field_name = str(auto_arg_index)
                    auto_arg_index += 1
                elif field_name.isdigit():
                    auto_arg_index = False
                obj, arg_used = self.get_field(field_name, args, kwargs)
                used_args.add(arg_used)
                obj = self.convert_field(obj, conversion)
                format_spec, auto_arg_index = self._vformat(format_spec, args, kwargs, used_args, recursion_depth - 1, auto_arg_index=auto_arg_index)
                result.append(self.format_field(obj, format_spec))
        return ''.join(result), auto_arg_index
    def get_value(self, key, args, kwargs):
        if isinstance(key, int):
            return args[key]
        return kwargs[key]
    def check_unused_args(self, used_args, args, kwargs):
        pass
    def format_field(self, value, format_spec):
        return format(value, format_spec)
    def convert_field(self, value, conversion):
        if conversion is None:
            return value
        elif conversion == 's':
            return str(value)
        elif conversion == 'r':
            return repr(value)
        elif conversion == 'a':
            return ascii(value)
        raise ValueError("Unknown conversion specifier {0!s}".format(conversion))
    def parse(self, format_string):
        out = []
        i = 0
        n = len(format_string)
        lit = []
        while i < n:
            c = format_string[i]
            if c == '{':
                if i + 1 < n and format_string[i + 1] == '{':
                    lit.append('{')
                    i += 2
                    continue
                depth = 1
                j = i + 1
                while j < n and depth:
                    if format_string[j] == '{':
                        depth += 1
                    elif format_string[j] == '}':
                        depth -= 1
                    j += 1
                if depth:
                    raise ValueError("expected '}' before end of string")
                field = format_string[i + 1:j - 1]
                conv = None
                spec = ''
                br = 0
                end = len(field)
                for k, ch in enumerate(field):
                    if ch == '[':
                        br += 1
                    elif ch == ']':
                        br -= 1
                    elif ch in '!:' and br == 0:
                        end = k
                        break
                name = field[:end]
                rest = field[end:]
                if rest.startswith('!'):
                    conv = rest[1:2]
                    rest = rest[2:]
                if rest.startswith(':'):
                    spec = rest[1:]
                out.append((''.join(lit), name, spec, conv))
                lit = []
                i = j
                continue
            if c == '}':
                if i + 1 < n and format_string[i + 1] == '}':
                    lit.append('}')
                    i += 2
                    continue
                raise ValueError("Single '}' encountered in format string")
            lit.append(c)
            i += 1
        if lit:
            out.append((''.join(lit), None, None, None))
        return out
    def get_field(self, field_name, args, kwargs):
        first, rest = _split_field(field_name)
        obj = self.get_value(first, args, kwargs)
        for is_attr, i in rest:
            if is_attr:
                obj = getattr(obj, i)
            else:
                obj = obj[i]
        return obj, first

def _split_field(name):
    i = 0
    while i < len(name) and name[i] not in '.[':
        i += 1
    first = name[:i]
    if first.isdigit():
        first = int(first)
    rest = []
    while i < len(name):
        if name[i] == '.':
            j = i + 1
            while j < len(name) and name[j] not in '.[':
                j += 1
            rest.append((True, name[i + 1:j]))
            i = j
        else:
            j = name.index(']', i)
            k = name[i + 1:j]
            if k.isdigit():
                k = int(k)
            rest.append((False, k))
            i = j + 1
    return first, rest
"##;

const KEYWORD: &str = r##"
kwlist = ['False', 'None', 'True', 'and', 'as', 'assert', 'async', 'await', 'break', 'class', 'continue', 'def', 'del', 'elif', 'else', 'except', 'finally', 'for', 'from', 'global', 'if', 'import', 'in', 'is', 'lambda', 'nonlocal', 'not', 'or', 'pass', 'raise', 'return', 'try', 'while', 'with', 'yield']
softkwlist = ['_', 'case', 'match', 'type']
_kw = frozenset(kwlist)
def iskeyword(s):
    return s in _kw
def issoftkeyword(s):
    return s in softkwlist
"##;

const FUTURE: &str = r##"
class _Feature:
    def __init__(self, optionalRelease, mandatoryRelease, compiler_flag):
        self.optional = optionalRelease
        self.mandatory = mandatoryRelease
        self.compiler_flag = compiler_flag
    def getOptionalRelease(self):
        return self.optional
    def getMandatoryRelease(self):
        return self.mandatory
nested_scopes = _Feature((2, 1, 0, "beta", 1), (2, 2, 0, "alpha", 0), 0x0010)
generators = _Feature((2, 2, 0, "alpha", 1), (2, 3, 0, "final", 0), 0)
division = _Feature((2, 2, 0, "alpha", 2), (3, 0, 0, "alpha", 0), 0x2000)
absolute_import = _Feature((2, 5, 0, "alpha", 1), (3, 0, 0, "alpha", 0), 0x4000)
with_statement = _Feature((2, 5, 0, "alpha", 1), (2, 6, 0, "alpha", 0), 0x8000)
print_function = _Feature((2, 6, 0, "alpha", 2), (3, 0, 0, "alpha", 0), 0x10000)
unicode_literals = _Feature((2, 6, 0, "alpha", 2), (3, 0, 0, "alpha", 0), 0x20000)
barry_as_FLUFL = _Feature((3, 1, 0, "alpha", 2), (4, 0, 0, "alpha", 0), 0x40000)
generator_stop = _Feature((3, 5, 0, "beta", 1), (3, 7, 0, "alpha", 0), 0x80000)
annotations = _Feature((3, 7, 0, "beta", 1), None, 0x1000000)
CO_FUTURE_DIVISION = 0x2000
CO_FUTURE_ABSOLUTE_IMPORT = 0x4000
CO_FUTURE_WITH_STATEMENT = 0x8000
CO_FUTURE_PRINT_FUNCTION = 0x10000
CO_FUTURE_UNICODE_LITERALS = 0x20000
all_feature_names = ["nested_scopes", "generators", "division", "absolute_import", "with_statement", "print_function", "unicode_literals", "barry_as_FLUFL", "generator_stop", "annotations"]
"##;

const CONTEXTLIB: &str = r##"
import sys
from functools import wraps

class AbstractContextManager(object):
    def __enter__(self):
        return self
    def __exit__(self, exc_type, exc_value, traceback):
        return None

class ContextDecorator(object):
    def _recreate_cm(self):
        return self
    def __call__(self, func):
        @wraps(func)
        def inner(*args, **kwds):
            with self._recreate_cm():
                return func(*args, **kwds)
        return inner

class _GeneratorContextManager(ContextDecorator):
    def __init__(self, func, args, kwds):
        self.gen = func(*args, **kwds)
        self.func, self.args, self.kwds = func, args, kwds
    def _recreate_cm(self):
        return self.__class__(self.func, self.args, self.kwds)
    def __enter__(self):
        try:
            return next(self.gen)
        except StopIteration:
            raise RuntimeError("generator didn't yield")
    def __exit__(self, typ, value, traceback):
        if typ is None:
            try:
                next(self.gen)
            except StopIteration:
                return False
            else:
                raise RuntimeError("generator didn't stop")
        else:
            if value is None:
                value = typ()
            try:
                self.gen.throw(typ, value, traceback)
            except StopIteration as exc:
                return exc is not value
            except RuntimeError as exc:
                if exc is value:
                    return False
                raise
            except BaseException as exc:
                if exc is value:
                    return False
                raise
            raise RuntimeError("generator didn't stop after throw()")

def contextmanager(func):
    @wraps(func)
    def helper(*args, **kwds):
        return _GeneratorContextManager(func, args, kwds)
    return helper

class closing(AbstractContextManager):
    def __init__(self, thing):
        self.thing = thing
    def __enter__(self):
        return self.thing
    def __exit__(self, *exc_info):
        self.thing.close()

class suppress(AbstractContextManager):
    def __init__(self, *exceptions):
        self._exceptions = exceptions
    def __enter__(self):
        pass
    def __exit__(self, exctype, excinst, exctb):
        return exctype is not None and issubclass(exctype, self._exceptions)

class nullcontext(AbstractContextManager):
    def __init__(self, enter_result=None):
        self.enter_result = enter_result
    def __enter__(self):
        return self.enter_result
    def __exit__(self, *excinfo):
        pass

class _RedirectStream(AbstractContextManager):
    _stream = None
    def __init__(self, new_target):
        self._new_target = new_target
        self._old_targets = []
    def __enter__(self):
        self._old_targets.append(getattr(sys, self._stream))
        setattr(sys, self._stream, self._new_target)
        return self._new_target
    def __exit__(self, exctype, excinst, exctb):
        setattr(sys, self._stream, self._old_targets.pop())

class redirect_stdout(_RedirectStream):
    _stream = "stdout"

class redirect_stderr(_RedirectStream):
    _stream = "stderr"

class ExitStack(AbstractContextManager):
    def __init__(self):
        self._exit_callbacks = []
    def push(self, exit):
        self._exit_callbacks.append(exit)
        return exit
    def callback(self, callback, *args, **kwds):
        def _exit_wrapper(exc_type, exc, tb):
            callback(*args, **kwds)
        self._exit_callbacks.append(_exit_wrapper)
        return callback
    def enter_context(self, cm):
        result = cm.__enter__()
        self._exit_callbacks.append(cm.__exit__)
        return result
    def close(self):
        self.__exit__(None, None, None)
    def pop_all(self):
        new = ExitStack()
        new._exit_callbacks = self._exit_callbacks
        self._exit_callbacks = []
        return new
    def __exit__(self, *exc_details):
        suppressed = False
        while self._exit_callbacks:
            cb = self._exit_callbacks.pop()
            if cb(*exc_details):
                suppressed = True
                exc_details = (None, None, None)
        return suppressed
"##;

const ENUM: &str = r##"
import sys

class _auto_null:
    pass

class auto:
    _counter = 0
    def __init__(self):
        auto._counter += 1
        self._order = auto._counter
        self.value = _auto_null

class _EnumDict(dict):
    def __init__(self):
        dict.__init__(self)
        self._member_names = []
        self._last_values = []
    def __setitem__(self, key, value):
        if key.startswith('__') and key.endswith('__') or key.startswith('_') and key not in ('_generate_next_value_',) and key.endswith('_') or callable(value) and not isinstance(value, auto) or isinstance(value, (classmethod, staticmethod, property)):
            dict.__setitem__(self, key, value)
            return
        if key.startswith('_'):
            dict.__setitem__(self, key, value)
            return
        if key in self._member_names:
            raise TypeError('Attempted to reuse key: %r' % key)
        if isinstance(value, auto):
            gen = self.get('_generate_next_value_', None)
            if gen is None:
                value = len(self._member_names) + 1 if not self._last_values else (self._last_values[-1] + 1 if isinstance(self._last_values[-1], int) else len(self._member_names) + 1)
            else:
                value = gen(key, 1, len(self._member_names), self._last_values[:])
        self._member_names.append(key)
        self._last_values.append(value)
        dict.__setitem__(self, key, value)

class EnumMeta(type):
    @classmethod
    def __prepare__(metacls, cls, bases, **kwds):
        return _EnumDict()
    def __new__(metacls, cls, bases, classdict, **kwds):
        member_names = list(getattr(classdict, '_member_names', []))
        values = {}
        for k in member_names:
            values[k] = dict.__getitem__(classdict, k)
        ns = dict(classdict)
        for k in member_names:
            del ns[k]
        enum_class = type.__new__(metacls, cls, bases, ns)
        member_type = int if any(issubclass(b, int) for b in bases) else (str if any(issubclass(b, str) for b in bases) else object)
        enum_class._member_names_ = []
        enum_class._member_map_ = {}
        enum_class._value2member_map_ = {}
        enum_class._member_type_ = member_type
        for name in member_names:
            value = values[name]
            args = value if isinstance(value, tuple) else (value,)
            if member_type is object:
                member = object.__new__(enum_class)
                if '__init__' in ns or any('__init__' in b.__dict__ for b in bases if b is not object and b not in (Enum,)):
                    enum_class.__init__(member, *args)
                member._value_ = value
            else:
                member = member_type.__new__(enum_class, *args)
                member._value_ = member_type(*args)
            member._name_ = name
            member.__objclass__ = enum_class
            if value in enum_class._value2member_map_:
                canonical = enum_class._value2member_map_[value]
                enum_class._member_map_[name] = canonical
                type.__setattr__(enum_class, name, canonical)
                continue
            enum_class._member_names_.append(name)
            enum_class._member_map_[name] = member
            try:
                enum_class._value2member_map_[member._value_] = member
            except TypeError:
                pass
            type.__setattr__(enum_class, name, member)
        return enum_class
    def __call__(cls, value, names=None, module=None, qualname=None, type=None, start=1):
        if names is None:
            return cls.__new_member__(value)
        if isinstance(names, str):
            names = names.replace(',', ' ').split()
        if names and isinstance(names[0], str) and not isinstance(names, dict):
            items = [(n, i + start) for i, n in enumerate(names)] if not isinstance(names[0], tuple) else names
        elif isinstance(names, dict):
            items = list(names.items())
        else:
            items = list(names)
        bases = (cls,) if type is None else (type, cls)
        d = _EnumDict()
        for k, v in items:
            d[k] = v
        return EnumMeta.__new__(EnumMeta, value, bases, d)
    def __new_member__(cls, value):
        if isinstance(value, cls):
            return value
        try:
            return cls._value2member_map_[value]
        except (KeyError, TypeError):
            for m in cls._member_map_.values():
                if m._value_ == value:
                    return m
        missing = getattr(cls, '_missing_', None)
        if missing is not None:
            r = missing(value)
            if r is not None:
                return r
        raise ValueError('%r is not a valid %s' % (value, cls.__name__))
    def __getitem__(cls, name):
        return cls._member_map_[name]
    def __iter__(cls):
        return (cls._member_map_[name] for name in cls._member_names_)
    def __reversed__(cls):
        return (cls._member_map_[name] for name in reversed(cls._member_names_))
    def __len__(cls):
        return len(cls._member_names_)
    def __contains__(cls, member):
        return isinstance(member, cls) and member._name_ in cls._member_map_
    def __repr__(cls):
        return "<enum %r>" % cls.__name__
    @property
    def __members__(cls):
        return dict(cls._member_map_)
    def __bool__(cls):
        return True

EnumType = EnumMeta

class Enum(metaclass=EnumMeta):
    def __repr__(self):
        return "<%s.%s: %r>" % (self.__class__.__name__, self._name_, self._value_)
    def __str__(self):
        return "%s.%s" % (self.__class__.__name__, self._name_)
    def __format__(self, format_spec):
        if self._member_type_ is object:
            return format(str(self), format_spec)
        return format(self._value_, format_spec)
    def __hash__(self):
        return hash(self._name_)
    def __reduce_ex__(self, proto):
        return self.__class__, (self._value_,)
    @property
    def name(self):
        return self._name_
    @property
    def value(self):
        return self._value_
    def __dir__(self):
        return ['__class__', '__doc__', '__module__', 'name', 'value']

class IntEnum(int, Enum):
    def __str__(self):
        return int.__repr__(self._value_)

class StrEnum(str, Enum):
    def __str__(self):
        return self._value_
    @staticmethod
    def _generate_next_value_(name, start, count, last_values):
        return name.lower()

class Flag(Enum):
    @staticmethod
    def _generate_next_value_(name, start, count, last_values):
        if not count:
            return start if start is not None else 1
        high = 0
        for v in last_values:
            high = max(high, v.bit_length())
        return 2 ** high
    @classmethod
    def _missing_(cls, value):
        pseudo = object.__new__(cls)
        pseudo._name_ = None
        pseudo._value_ = value
        return pseudo
    def __contains__(self, other):
        return other._value_ & self._value_ == other._value_
    def __or__(self, other):
        return self.__class__(self._value_ | other._value_)
    def __and__(self, other):
        return self.__class__(self._value_ & other._value_)
    def __xor__(self, other):
        return self.__class__(self._value_ ^ other._value_)
    def __bool__(self):
        return bool(self._value_)

class IntFlag(int, Flag):
    pass

def unique(enumeration):
    return enumeration

def _simple_enum(etype=Enum, **kwds):
    def decorator(cls):
        return cls
    return decorator
"##;

const HEAPQ: &str = r##"
def heappush(heap, item):
    heap.append(item)
    _siftdown(heap, 0, len(heap) - 1)

def heappop(heap):
    lastelt = heap.pop()
    if heap:
        returnitem = heap[0]
        heap[0] = lastelt
        _siftup(heap, 0)
        return returnitem
    return lastelt

def heapreplace(heap, item):
    returnitem = heap[0]
    heap[0] = item
    _siftup(heap, 0)
    return returnitem

def heappushpop(heap, item):
    if heap and heap[0] < item:
        item, heap[0] = heap[0], item
        _siftup(heap, 0)
    return item

def heapify(x):
    n = len(x)
    for i in reversed(range(n // 2)):
        _siftup(x, i)

def _siftdown(heap, startpos, pos):
    newitem = heap[pos]
    while pos > startpos:
        parentpos = (pos - 1) >> 1
        parent = heap[parentpos]
        if newitem < parent:
            heap[pos] = parent
            pos = parentpos
            continue
        break
    heap[pos] = newitem

def _siftup(heap, pos):
    endpos = len(heap)
    startpos = pos
    newitem = heap[pos]
    childpos = 2 * pos + 1
    while childpos < endpos:
        rightpos = childpos + 1
        if rightpos < endpos and not heap[childpos] < heap[rightpos]:
            childpos = rightpos
        heap[pos] = heap[childpos]
        pos = childpos
        childpos = 2 * pos + 1
    heap[pos] = newitem
    _siftdown(heap, startpos, pos)

def nlargest(n, iterable, key=None):
    return sorted(iterable, key=key, reverse=True)[:n]

def nsmallest(n, iterable, key=None):
    return sorted(iterable, key=key)[:n]

def merge(*iterables, key=None, reverse=False):
    out = []
    for it in iterables:
        out.extend(it)
    return iter(sorted(out, key=key, reverse=reverse))
"##;

const BISECT: &str = r##"
def bisect_right(a, x, lo=0, hi=None, *, key=None):
    if lo < 0:
        raise ValueError('lo must be non-negative')
    if hi is None:
        hi = len(a)
    while lo < hi:
        mid = (lo + hi) // 2
        v = a[mid] if key is None else key(a[mid])
        if x < v:
            hi = mid
        else:
            lo = mid + 1
    return lo

def bisect_left(a, x, lo=0, hi=None, *, key=None):
    if lo < 0:
        raise ValueError('lo must be non-negative')
    if hi is None:
        hi = len(a)
    while lo < hi:
        mid = (lo + hi) // 2
        v = a[mid] if key is None else key(a[mid])
        if v < x:
            lo = mid + 1
        else:
            hi = mid
    return lo

def insort_right(a, x, lo=0, hi=None, *, key=None):
    lo = bisect_right(a, x if key is None else key(x), lo, hi, key=key)
    a.insert(lo, x)

def insort_left(a, x, lo=0, hi=None, *, key=None):
    lo = bisect_left(a, x if key is None else key(x), lo, hi, key=key)
    a.insert(lo, x)

bisect = bisect_right
insort = insort_right
"##;

const TEXTWRAP: &str = r##"
def wrap(text, width=70, **kwargs):
    words = text.split()
    lines = []
    cur = ''
    for w in words:
        if not cur:
            cur = w
        elif len(cur) + 1 + len(w) <= width:
            cur += ' ' + w
        else:
            lines.append(cur)
            cur = w
    if cur:
        lines.append(cur)
    return lines

def fill(text, width=70, **kwargs):
    return '\n'.join(wrap(text, width, **kwargs))

def shorten(text, width, placeholder=' [...]', **kwargs):
    text = ' '.join(text.split())
    if len(text) <= width:
        return text
    words = text.split()
    out = ''
    for w in words:
        cand = (out + ' ' + w) if out else w
        if len(cand) + len(placeholder) > width:
            break
        out = cand
    return out + placeholder

def dedent(text):
    lines = text.split('\n')
    margin = None
    for l in lines:
        s = l.lstrip()
        if not s:
            continue
        ind = l[:len(l) - len(s)]
        if margin is None:
            margin = ind
        else:
            while not ind.startswith(margin):
                margin = margin[:-1]
    if not margin:
        return '\n'.join(l if l.strip() else '' for l in lines) if margin is not None else text
    return '\n'.join(l[len(margin):] if l.startswith(margin) else l.strip() and l or '' for l in lines)

def indent(text, prefix, predicate=None):
    if predicate is None:
        predicate = lambda line: line.strip()
    return ''.join(prefix + line if predicate(line) else line for line in text.splitlines(True))

class TextWrapper:
    def __init__(self, width=70, **kwargs):
        self.width = width
        for k, v in kwargs.items():
            setattr(self, k, v)
    def wrap(self, text):
        return wrap(text, self.width)
    def fill(self, text):
        return fill(text, self.width)
"##;

const WEAKREF: &str = r##"
import sys
from _weakref import (getweakrefcount, getweakrefs, ref, proxy, CallableProxyType,
                      ProxyType, ReferenceType, _remove_dead_weakref)
from _collections_abc import MutableMapping

ProxyTypes = (ProxyType, CallableProxyType)

__all__ = ["ref", "proxy", "getweakrefcount", "getweakrefs", "WeakKeyDictionary",
           "ReferenceType", "ProxyType", "CallableProxyType", "ProxyTypes",
           "WeakValueDictionary", "WeakSet", "WeakMethod", "finalize"]

# Python 2 callers expect list-returning mapping methods.
_PY2 = sys.version_info[0] < 3

def _result(gen):
    return list(gen) if _PY2 else gen


class WeakMethod(ref):
    __slots__ = "_func_ref", "_meth_type", "_alive", "__weakref__"

    def __new__(cls, meth, callback=None):
        try:
            obj = meth.__self__
            func = meth.__func__
        except AttributeError:
            raise TypeError("argument should be a bound method, not {}".format(type(meth))) from None
        def _cb(arg):
            self = self_wr()
            if self._alive:
                self._alive = False
                if callback is not None:
                    callback(self)
        self = ref.__new__(cls, obj, _cb)
        self._func_ref = ref(func, _cb)
        self._meth_type = type(meth)
        self._alive = True
        self_wr = ref(self)
        return self

    def __call__(self):
        obj = ref.__call__(self)
        func = self._func_ref()
        if obj is None or func is None:
            return None
        return func.__get__(obj, type(obj))

    def __eq__(self, other):
        if isinstance(other, WeakMethod):
            if not self._alive or not other._alive:
                return self is other
            return ref.__eq__(self, other) and self._func_ref == other._func_ref
        return NotImplemented

    def __ne__(self, other):
        if isinstance(other, WeakMethod):
            if not self._alive or not other._alive:
                return self is not other
            return ref.__ne__(self, other) or self._func_ref != other._func_ref
        return NotImplemented

    __hash__ = ref.__hash__


class KeyedRef(ref):
    """A weakref carrying the dictionary key it is stored under."""

    __slots__ = "key",

    def __new__(type, ob, callback, key):
        self = ref.__new__(type, ob, callback)
        self.key = key
        return self

    def __init__(self, ob, callback, key):
        ref.__init__(self, ob, callback)


class WeakValueDictionary(MutableMapping):
    """Mapping whose entries disappear when their values are collected."""

    def __init__(self, other=(), **kw):
        def remove(wr, selfref=ref(self), _atomic_removal=_remove_dead_weakref):
            self = selfref()
            if self is not None:
                _atomic_removal(self.data, wr.key)
        self._remove = remove
        self.data = {}
        self.update(other, **kw)

    def __getitem__(self, key):
        o = self.data[key]()
        if o is None:
            raise KeyError(key)
        return o

    def __delitem__(self, key):
        del self.data[key]

    def __len__(self):
        return len(self.data)

    def __contains__(self, key):
        try:
            o = self.data[key]()
        except KeyError:
            return False
        return o is not None

    def __repr__(self):
        return "<%s at %#x>" % (self.__class__.__name__, id(self))

    def __setitem__(self, key, value):
        self.data[key] = KeyedRef(value, self._remove, key)

    def copy(self):
        new = WeakValueDictionary()
        for key, wr in list(self.data.items()):
            o = wr()
            if o is not None:
                new[key] = o
        return new

    __copy__ = copy

    def __deepcopy__(self, memo):
        from copy import deepcopy
        new = self.__class__()
        for key, wr in list(self.data.items()):
            o = wr()
            if o is not None:
                new[deepcopy(key, memo)] = o
        return new

    def get(self, key, default=None):
        try:
            wr = self.data[key]
        except KeyError:
            return default
        o = wr()
        if o is None:
            return default
        return o

    def _items(self):
        for k, wr in list(self.data.items()):
            v = wr()
            if v is not None:
                yield k, v

    def _keys(self):
        for k, wr in list(self.data.items()):
            if wr() is not None:
                yield k

    def _values(self):
        for wr in list(self.data.values()):
            obj = wr()
            if obj is not None:
                yield obj

    def items(self):
        return _result(self._items())

    def keys(self):
        return _result(self._keys())

    def values(self):
        return _result(self._values())

    def __iter__(self):
        return self._keys()

    iteritems = _items
    iterkeys = _keys
    itervalues = _values

    def has_key(self, key):
        return key in self

    def itervaluerefs(self):
        for wr in list(self.data.values()):
            yield wr

    def valuerefs(self):
        return list(self.data.values())

    def popitem(self):
        while True:
            key, wr = self.data.popitem()
            o = wr()
            if o is not None:
                return key, o

    def pop(self, key, *args):
        try:
            o = self.data.pop(key)()
        except KeyError:
            o = None
        if o is None:
            if args:
                return args[0]
            raise KeyError(key)
        return o

    def setdefault(self, key, default=None):
        try:
            o = self.data[key]()
        except KeyError:
            o = None
        if o is None:
            self.data[key] = KeyedRef(default, self._remove, key)
            return default
        return o

    def update(self, other=None, **kwargs):
        d = self.data
        if other is not None:
            if not hasattr(other, "items"):
                other = dict(other)
            for key, o in list(other.items()):
                d[key] = KeyedRef(o, self._remove, key)
        for key, o in kwargs.items():
            d[key] = KeyedRef(o, self._remove, key)

    def clear(self):
        self.data.clear()

    def __or__(self, other):
        if isinstance(other, (dict, MutableMapping)):
            c = self.copy()
            c.update(other)
            return c
        return NotImplemented

    def __ior__(self, other):
        self.update(other)
        return self


class WeakKeyDictionary(MutableMapping):
    """Mapping whose entries disappear when their keys are collected."""

    def __init__(self, dict=None):
        self.data = {}
        def remove(k, selfref=ref(self)):
            self = selfref()
            if self is not None:
                try:
                    del self.data[k]
                except KeyError:
                    pass
        self._remove = remove
        if dict is not None:
            self.update(dict)

    def __delitem__(self, key):
        del self.data[ref(key)]

    def __getitem__(self, key):
        return self.data[ref(key)]

    def __len__(self):
        return len(self.data)

    def __repr__(self):
        return "<%s at %#x>" % (self.__class__.__name__, id(self))

    def __setitem__(self, key, value):
        self.data[ref(key, self._remove)] = value

    def copy(self):
        new = WeakKeyDictionary()
        for key, value in list(self.data.items()):
            o = key()
            if o is not None:
                new[o] = value
        return new

    __copy__ = copy

    def __deepcopy__(self, memo):
        from copy import deepcopy
        new = self.__class__()
        for key, value in list(self.data.items()):
            o = key()
            if o is not None:
                new[o] = deepcopy(value, memo)
        return new

    def get(self, key, default=None):
        return self.data.get(ref(key), default)

    def __contains__(self, key):
        try:
            wr = ref(key)
        except TypeError:
            return False
        return wr in self.data

    def _items(self):
        for wr, value in list(self.data.items()):
            key = wr()
            if key is not None:
                yield key, value

    def _keys(self):
        for wr in list(self.data):
            obj = wr()
            if obj is not None:
                yield obj

    def _values(self):
        for wr, value in list(self.data.items()):
            if wr() is not None:
                yield value

    def items(self):
        return _result(self._items())

    def keys(self):
        return _result(self._keys())

    def values(self):
        return _result(self._values())

    def __iter__(self):
        return self._keys()

    iteritems = _items
    iterkeys = _keys
    itervalues = _values

    def has_key(self, key):
        return key in self

    def keyrefs(self):
        return list(self.data)

    def popitem(self):
        while True:
            key, value = self.data.popitem()
            o = key()
            if o is not None:
                return o, value

    def pop(self, key, *args):
        return self.data.pop(ref(key), *args)

    def setdefault(self, key, default=None):
        return self.data.setdefault(ref(key, self._remove), default)

    def update(self, dict=None, **kwargs):
        d = self.data
        if dict is not None:
            if not hasattr(dict, "items"):
                dict = type({})(dict)
            for key, value in list(dict.items()):
                d[ref(key, self._remove)] = value
        if len(kwargs):
            self.update(kwargs)

    def clear(self):
        self.data.clear()

    def __or__(self, other):
        if isinstance(other, (dict, MutableMapping)):
            c = self.copy()
            c.update(other)
            return c
        return NotImplemented

    def __ior__(self, other):
        self.update(other)
        return self


class WeakSet(object):
    """Set whose members disappear when they are collected."""

    def __init__(self, data=None):
        self.data = set()
        def _remove(item, selfref=ref(self)):
            self = selfref()
            if self is not None:
                self.data.discard(item)
        self._remove = _remove
        if data is not None:
            self.update(data)

    def __iter__(self):
        for itemref in list(self.data):
            item = itemref()
            if item is not None:
                yield item

    def __len__(self):
        return len(self.data)

    def __contains__(self, item):
        try:
            wr = ref(item)
        except TypeError:
            return False
        return wr in self.data

    def __reduce__(self):
        return (self.__class__, (list(self),), getattr(self, '__dict__', None))

    def add(self, item):
        self.data.add(ref(item, self._remove))

    def clear(self):
        self.data.clear()

    def copy(self):
        return self.__class__(self)

    def pop(self):
        while True:
            try:
                itemref = self.data.pop()
            except KeyError:
                raise KeyError('pop from empty WeakSet') from None
            item = itemref()
            if item is not None:
                return item

    def remove(self, item):
        self.data.remove(ref(item))

    def discard(self, item):
        self.data.discard(ref(item))

    def update(self, other):
        for element in other:
            self.add(element)

    def __ior__(self, other):
        self.update(other)
        return self

    def difference(self, other):
        newset = self.copy()
        newset.difference_update(other)
        return newset
    __sub__ = difference

    def difference_update(self, other):
        self.__isub__(other)

    def __isub__(self, other):
        if self is other:
            self.data.clear()
        else:
            self.data.difference_update([ref(item) for item in other])
        return self

    def intersection(self, other):
        return self.__class__([item for item in other if item in self])
    __and__ = intersection

    def intersection_update(self, other):
        self.__iand__(other)

    def __iand__(self, other):
        self.data.intersection_update([ref(item) for item in other])
        return self

    def issubset(self, other):
        return self.data.issubset([ref(item) for item in other])
    __le__ = issubset

    def __lt__(self, other):
        return self.data < set(map(ref, other))

    def issuperset(self, other):
        return self.data.issuperset([ref(item) for item in other])
    __ge__ = issuperset

    def __gt__(self, other):
        return self.data > set(map(ref, other))

    def __eq__(self, other):
        if not isinstance(other, self.__class__):
            return NotImplemented
        return self.data == set(map(ref, other))

    def __ne__(self, other):
        r = self.__eq__(other)
        return r if r is NotImplemented else not r

    def symmetric_difference(self, other):
        newset = self.copy()
        newset.symmetric_difference_update(other)
        return newset
    __xor__ = symmetric_difference

    def symmetric_difference_update(self, other):
        self.__ixor__(other)

    def __ixor__(self, other):
        if self is other:
            self.data.clear()
        else:
            self.data.symmetric_difference_update([ref(item, self._remove) for item in other])
        return self

    def union(self, other):
        return self.__class__([e for s in (self, other) for e in s])
    __or__ = union

    def isdisjoint(self, other):
        return len(self.intersection(other)) == 0

    def __repr__(self):
        return repr(self.data)

    __hash__ = None


class _FinalizeInfo(object):
    __slots__ = ("weakref", "func", "args", "kwargs", "atexit", "index")


class finalize(object):
    """Calls func(*args, **kwargs) once obj is collected (or at exit)."""

    __slots__ = ()
    _registry = {}
    _shutdown = False
    _index = [0]
    _dirty = False
    _registered_with_atexit = False
    _Info = _FinalizeInfo

    def __init__(self, obj, func, *args, **kwargs):
        if not finalize._registered_with_atexit:
            import atexit
            atexit.register(finalize._exitfunc)
            finalize._registered_with_atexit = True
        info = _FinalizeInfo()
        info.weakref = ref(obj, self)
        info.func = func
        info.args = args
        info.kwargs = kwargs or None
        info.atexit = True
        info.index = finalize._index[0]
        finalize._index[0] += 1
        finalize._registry[self] = info
        finalize._dirty = True

    def __call__(self, _=None):
        info = finalize._registry.pop(self, None)
        if info and not finalize._shutdown:
            return info.func(*info.args, **(info.kwargs or {}))

    def detach(self):
        info = finalize._registry.get(self)
        obj = info and info.weakref()
        if obj is not None and finalize._registry.pop(self, None):
            return (obj, info.func, info.args, info.kwargs or {})

    def peek(self):
        info = finalize._registry.get(self)
        obj = info and info.weakref()
        if obj is not None:
            return (obj, info.func, info.args, info.kwargs or {})

    @property
    def alive(self):
        return self in finalize._registry

    @property
    def atexit(self):
        info = finalize._registry.get(self)
        return bool(info) and info.atexit

    @atexit.setter
    def atexit(self, value):
        info = finalize._registry.get(self)
        if info:
            info.atexit = bool(value)

    def __repr__(self):
        info = finalize._registry.get(self)
        obj = info and info.weakref()
        if obj is None:
            return '<%s object at %#x; dead>' % (type(self).__name__, id(self))
        return '<%s object at %#x; for %r at %#x>' % (type(self).__name__, id(self), type(obj).__name__, id(obj))

    @classmethod
    def _select_for_exit(cls):
        L = [(f, i) for (f, i) in list(cls._registry.items()) if i.atexit]
        L.sort(key=lambda item: item[1].index)
        return [f for (f, i) in L]

    @classmethod
    def _exitfunc(cls):
        reenable_gc = False
        try:
            if cls._registry:
                import gc
                if gc.isenabled():
                    reenable_gc = True
                    gc.disable()
                pending = None
                while True:
                    if pending is None or finalize._dirty:
                        pending = cls._select_for_exit()
                        finalize._dirty = False
                    if not pending:
                        break
                    f = pending.pop()
                    try:
                        f()
                    except Exception:
                        import traceback
                        traceback.print_exc()
        finally:
            finalize._shutdown = True
            if reenable_gc:
                gc.enable()
"##;

/// Handlers run when the embedding host calls `atexit._run_exitfuncs()`.
const ATEXIT: &str = r##"
_handlers = []
def register(f, *args, **kwargs):
    _handlers.append((f, args, kwargs))
    return f
def unregister(f):
    _handlers[:] = [h for h in _handlers if h[0] is not f]
def _clear():
    del _handlers[:]
def _ncallbacks():
    return len(_handlers)
def _run_exitfuncs():
    while _handlers:
        f, args, kwargs = _handlers.pop()
        try:
            f(*args, **kwargs)
        except SystemExit:
            pass
        except BaseException:
            import traceback
            traceback.print_exc()
"##;

const WARNINGS: &str = r##"
filters = []
def warn(message, category=None, stacklevel=1, source=None):
    pass
def warn_explicit(*args, **kwargs):
    pass
def simplefilter(action, category=Warning, lineno=0, append=False):
    pass
def filterwarnings(action, message="", category=Warning, module="", lineno=0, append=False):
    pass
def resetwarnings():
    pass
def showwarning(*args, **kwargs):
    pass
def formatwarning(message, category, filename, lineno, line=None):
    return "%s:%s: %s: %s\n" % (filename, lineno, category.__name__, message)
class catch_warnings(object):
    def __init__(self, record=False, module=None):
        self._record = record
    def __enter__(self):
        return [] if self._record else None
    def __exit__(self, *exc_info):
        pass
"##;

const TRACEBACK: &str = r##"
import sys

def _entries(tb):
    return tb if isinstance(tb, list) else []

def format_tb(tb, limit=None):
    out = []
    for (filename, lineno, name) in _entries(tb):
        out.append('  File "%s", line %d, in %s\n' % (filename, lineno, name))
    return out

def extract_tb(tb, limit=None):
    return [(f, l, n, None) for (f, l, n) in _entries(tb)]

def format_exception_only(etype, value=None):
    if value is None and isinstance(etype, BaseException):
        value = etype
        etype = type(value)
    msg = str(value)
    name = etype.__name__ if etype is not None else 'None'
    if msg:
        return ['%s: %s\n' % (name, msg)]
    return ['%s\n' % name]

def format_exception(etype, value=None, tb=None, limit=None, chain=True):
    if value is None and isinstance(etype, BaseException):
        value = etype
        etype = type(value)
        tb = getattr(value, '__traceback__', None)
    out = []
    if chain and value is not None:
        cause = getattr(value, '__cause__', None)
        context = getattr(value, '__context__', None)
        if cause is not None:
            out.extend(format_exception(type(cause), cause, getattr(cause, '__traceback__', None)))
            out.append('\nThe above exception was the direct cause of the following exception:\n\n')
        elif context is not None and not getattr(value, '__suppress_context__', False):
            out.extend(format_exception(type(context), context, getattr(context, '__traceback__', None)))
            out.append('\nDuring handling of the above exception, another exception occurred:\n\n')
    out.append('Traceback (most recent call last):\n')
    out.extend(format_tb(tb))
    out.extend(format_exception_only(etype, value))
    return out

def format_exc(limit=None, chain=True):
    return ''.join(format_exception(*sys.exc_info(), limit=limit, chain=chain))

def print_exc(limit=None, file=None, chain=True):
    if file is None:
        file = sys.stderr
    file.write(format_exc(limit, chain))

def print_exception(etype, value=None, tb=None, limit=None, file=None, chain=True):
    if file is None:
        file = sys.stderr
    file.write(''.join(format_exception(etype, value, tb, limit, chain)))

def format_stack(f=None, limit=None):
    return []

def extract_stack(f=None, limit=None):
    return []

def print_stack(f=None, limit=None, file=None):
    pass

def format_list(extracted_list):
    return ['  File "%s", line %d, in %s\n' % (f, l, n) for (f, l, n, _) in extracted_list]

def walk_tb(tb):
    return iter([])
"##;

const RE: &str = r##"
from _sre import compile, match, search, fullmatch, findall, finditer, split, sub, subn, escape, purge, error
from _sre import Pattern, Match
from _sre import I, IGNORECASE, L, LOCALE, M, MULTILINE, S, DOTALL, U, UNICODE, X, VERBOSE, A, ASCII, DEBUG, T, TEMPLATE
_pattern_type = Pattern
class RegexFlag(int):
    pass
NOFLAG = 0
def template(pattern, flags=0):
    return compile(pattern, flags)
"##;

const SRE_COMPAT: &str = r##"
from _sre import error
MAXREPEAT = 4294967295
MAXGROUPS = 2147483647
"##;

const JSON: &str = r##"
import _json
from _json import JSONDecodeError

class JSONEncoder(object):
    item_separator = ', '
    key_separator = ': '
    def __init__(self, *, skipkeys=False, ensure_ascii=True, check_circular=True, allow_nan=True, sort_keys=False, indent=None, separators=None, default=None):
        self.skipkeys = skipkeys
        self.ensure_ascii = ensure_ascii
        self.allow_nan = allow_nan
        self.sort_keys = sort_keys
        self.indent = indent
        self.separators = separators
        if default is not None:
            self.default = default
    def default(self, o):
        raise TypeError('Object of type %s is not JSON serializable' % o.__class__.__name__)
    def encode(self, o):
        return _json.dumps(o, skipkeys=self.skipkeys, ensure_ascii=self.ensure_ascii, allow_nan=self.allow_nan, sort_keys=self.sort_keys, indent=self.indent, separators=self.separators, default=self.default)
    def iterencode(self, o, _one_shot=False):
        return iter([self.encode(o)])

class JSONDecoder(object):
    def __init__(self, *, object_hook=None, parse_float=None, parse_int=None, parse_constant=None, strict=True, object_pairs_hook=None):
        self.object_hook = object_hook
        self.object_pairs_hook = object_pairs_hook
    def decode(self, s):
        return _json.loads(s, object_hook=self.object_hook, object_pairs_hook=self.object_pairs_hook)
    def raw_decode(self, s, idx=0):
        return self.decode(s[idx:]), len(s)

def dumps(obj, *, skipkeys=False, ensure_ascii=True, check_circular=True, allow_nan=True, cls=None, indent=None, separators=None, default=None, sort_keys=False, **kw):
    if cls is not None:
        return cls(skipkeys=skipkeys, ensure_ascii=ensure_ascii, allow_nan=allow_nan, indent=indent, separators=separators, default=default, sort_keys=sort_keys, **kw).encode(obj)
    return _json.dumps(obj, skipkeys=skipkeys, ensure_ascii=ensure_ascii, allow_nan=allow_nan, indent=indent, separators=separators, default=default, sort_keys=sort_keys)

def dump(obj, fp, **kw):
    fp.write(dumps(obj, **kw))

def loads(s, *, cls=None, object_hook=None, parse_float=None, parse_int=None, parse_constant=None, object_pairs_hook=None, **kw):
    if cls is not None:
        return cls(object_hook=object_hook, object_pairs_hook=object_pairs_hook, **kw).decode(s)
    return _json.loads(s, object_hook=object_hook, object_pairs_hook=object_pairs_hook)

def load(fp, **kw):
    return loads(fp.read(), **kw)
"##;

const JSON_DECODER: &str = "from _json import JSONDecodeError\nfrom json import JSONDecoder\n";

const RANDOM: &str = r##"
from _random import Random as _Random
import math as _math

class Random(_Random):
    VERSION = 3

class SystemRandom(Random):
    pass

_inst = Random()
seed = _inst.seed
random = _inst.random
uniform = _inst.uniform
triangular = _inst.triangular
randint = _inst.randint
choice = _inst.choice
randrange = _inst.randrange
sample = _inst.sample
shuffle = _inst.shuffle
choices = _inst.choices
normalvariate = _inst.normalvariate
lognormvariate = _inst.lognormvariate
expovariate = _inst.expovariate
vonmisesvariate = _inst.vonmisesvariate
gammavariate = _inst.gammavariate
gauss = _inst.gauss
betavariate = _inst.betavariate
paretovariate = _inst.paretovariate
weibullvariate = _inst.weibullvariate
getstate = _inst.getstate
setstate = _inst.setstate
getrandbits = _inst.getrandbits
randbytes = _inst.randbytes
"##;

const IO: &str = r##"
from _io import *
from _io import StringIO, BytesIO, open, IOBase, RawIOBase, BufferedIOBase, TextIOBase, UnsupportedOperation, DEFAULT_BUFFER_SIZE
SEEK_SET = 0
SEEK_CUR = 1
SEEK_END = 2

def FileIO(file, mode='r', closefd=True, opener=None):
    if 'b' not in mode:
        mode = mode + 'b'
    return open(file, mode)

class _WrappedStream:
    def __init__(self, raw):
        self.raw = raw
        if hasattr(raw, 'name'):
            self.name = raw.name
        if hasattr(raw, 'mode'):
            self.mode = raw.mode

    @property
    def closed(self):
        return self.raw.closed

    def close(self):
        return self.raw.close()

    def flush(self):
        return self.raw.flush()

    def seek(self, offset, whence=0):
        return self.raw.seek(offset, whence)

    def tell(self):
        return self.raw.tell()

    def seekable(self):
        return self.raw.seekable() if hasattr(self.raw, 'seekable') else True

    def isatty(self):
        return self.raw.isatty() if hasattr(self.raw, 'isatty') else False

    def __enter__(self):
        if self.closed:
            raise ValueError("I/O operation on closed file.")
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()
        return False

class BufferedReader(_WrappedStream, BufferedIOBase):
    def __init__(self, raw, buffer_size=DEFAULT_BUFFER_SIZE):
        _WrappedStream.__init__(self, raw)

    def readable(self):
        return True

    def writable(self):
        return False

    def read(self, size=-1):
        return self.raw.read(size)

    def read1(self, size=-1):
        return self.raw.read(size)

    def readline(self, size=-1):
        return self.raw.readline(size)

    def readlines(self, hint=-1):
        return self.raw.readlines(hint)

    def __iter__(self):
        return self

    def __next__(self):
        line = self.readline()
        if not line:
            raise StopIteration
        return line

    def next(self):
        return self.__next__()

class BufferedWriter(_WrappedStream, BufferedIOBase):
    def __init__(self, raw, buffer_size=DEFAULT_BUFFER_SIZE):
        _WrappedStream.__init__(self, raw)

    def readable(self):
        return False

    def writable(self):
        return True

    def write(self, b):
        return self.raw.write(b)

    def writelines(self, lines):
        return self.raw.writelines(lines)

    def truncate(self, pos=None):
        return self.raw.truncate(pos)

class TextIOWrapper(TextIOBase):
    def __init__(self, buffer, encoding=None, errors=None, newline=None, line_buffering=False, write_through=False):
        self.buffer = buffer
        self.encoding = encoding or 'utf-8'
        self.errors = errors or 'strict'
        self.newline = newline
        self._buf = ''
        if hasattr(buffer, 'name'):
            self.name = buffer.name
        if hasattr(buffer, 'mode'):
            self.mode = buffer.mode

    @property
    def closed(self):
        return self.buffer.closed

    def close(self):
        self._buf = ''
        return self.buffer.close()

    def flush(self):
        return self.buffer.flush()

    def seekable(self):
        return self.buffer.seekable() if hasattr(self.buffer, 'seekable') else True

    def readable(self):
        return self.buffer.readable() if hasattr(self.buffer, 'readable') else True

    def writable(self):
        return self.buffer.writable() if hasattr(self.buffer, 'writable') else False

    def isatty(self):
        return self.buffer.isatty() if hasattr(self.buffer, 'isatty') else False

    def seek(self, offset, whence=0):
        self._buf = ''
        return self.buffer.seek(offset, whence)

    def tell(self):
        return self.buffer.tell()

    def _decode(self, data):
        s = data.decode(self.encoding, self.errors)
        if self.newline is None:
            s = s.replace('\r\n', '\n').replace('\r', '\n')
        return s

    def read(self, size=-1):
        if size is None or size < 0:
            rest = self.buffer.read(-1)
            out = self._buf + self._decode(rest)
            self._buf = ''
            return out
        while len(self._buf) < size:
            chunk = self.buffer.read(max(4096, size - len(self._buf)))
            if not chunk:
                break
            self._buf += self._decode(chunk)
        out = self._buf[:size]
        self._buf = self._buf[size:]
        return out

    def readline(self, size=-1):
        if self._buf:
            idx = self._buf.find('\n')
            if idx != -1:
                end = idx + 1 if (size is None or size < 0) else min(idx + 1, size)
                out = self._buf[:end]
                self._buf = self._buf[end:]
                return out
        raw = self.buffer.readline()
        text = self._buf + self._decode(raw)
        self._buf = ''
        if size is not None and size >= 0 and len(text) > size:
            self._buf = text[size:]
            return text[:size]
        return text

    def readlines(self, hint=-1):
        lines = []
        while True:
            line = self.readline()
            if not line:
                break
            lines.append(line)
        return lines

    def write(self, s):
        data = s.encode(self.encoding, self.errors)
        return self.buffer.write(data)

    def writelines(self, lines):
        for l in lines:
            self.write(l)

    def __iter__(self):
        return self

    def __next__(self):
        line = self.readline()
        if not line:
            raise StopIteration
        return line

    def next(self):
        return self.__next__()

    def __enter__(self):
        if self.closed:
            raise ValueError("I/O operation on closed file.")
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()
        return False
"##;

const OS: &str = r##"
from posix import *
from posix import environ, sep, altsep, extsep, pathsep, linesep, curdir, pardir, devnull, name, error
import posixpath as path
import sys
sys.modules['os.path'] = path
SEEK_SET = 0
SEEK_CUR = 1
SEEK_END = 2
F_OK = 0
R_OK = 4
W_OK = 2
X_OK = 1
def access(p, mode):
    return path.exists(p)
def walk(top, topdown=True, onerror=None, followlinks=False):
    try:
        names = listdir(top)
    except OSError as err:
        if onerror is not None:
            onerror(err)
        return
    dirs, nondirs = [], []
    for n in names:
        if path.isdir(path.join(top, n)):
            dirs.append(n)
        else:
            nondirs.append(n)
    if topdown:
        yield top, dirs, nondirs
    for d in dirs:
        for x in walk(path.join(top, d), topdown, onerror, followlinks):
            yield x
    if not topdown:
        yield top, dirs, nondirs
def stat(p):
    if not path.exists(p):
        raise FileNotFoundError(2, 'No such file or directory', p)
    class _st:
        st_size = path.getsize(p) if path.isfile(p) else 0
        st_mtime = 0.0
        st_mode = 0o40755 if path.isdir(p) else 0o100644
    return _st()
def fsencode(s):
    return s.encode('utf-8') if isinstance(s, str) else s
def fsdecode(s):
    return s.decode('utf-8') if isinstance(s, bytes) else s
def chdir(p):
    pass
def utime(p, times=None):
    pass
def rmdir(p):
    remove(p)
def removedirs(p):
    remove(p)
def getuid():
    return 0
def cpu_count():
    return 1
def scandir(p='.'):
    class _E:
        def __init__(self, d, n):
            self.name = n
            self.path = path.join(d, n)
        def is_dir(self):
            return path.isdir(self.path)
        def is_file(self):
            return path.isfile(self.path)
    return iter([_E(p, n) for n in listdir(p)])
PathLike = object
"##;

const OS_PATH: &str = r##"
from posix import _path_join as join, _path_split as split, _path_basename as basename, _path_dirname as dirname
from posix import _path_splitext as splitext, _path_normpath as normpath, _path_abspath as abspath, _path_isabs as isabs
from posix import _path_expanduser as expanduser, _path_exists as exists, _path_isfile as isfile, _path_isdir as isdir
from posix import _path_getsize as getsize, _path_getmtime as getmtime, _path_relpath as relpath, _path_commonprefix as commonprefix
sep = '/'
altsep = None
extsep = '.'
pathsep = ':'
curdir = '.'
pardir = '..'
def expandvars(p):
    return p
def realpath(p):
    return abspath(p)
def lexists(p):
    return exists(p)
def islink(p):
    return False
def ismount(p):
    return p == '/'
def normcase(p):
    return p
def getatime(p):
    return getmtime(p)
def getctime(p):
    return getmtime(p)
def splitdrive(p):
    return '', p
def samefile(a, b):
    return normpath(a) == normpath(b)
def commonpath(paths):
    parts = [normpath(p).split('/') for p in paths]
    out = []
    for group in zip(*parts):
        if all(x == group[0] for x in group):
            out.append(group[0])
        else:
            break
    return '/'.join(out) or ('/' if paths and paths[0].startswith('/') else '')
"##;

const BASE64: &str = r##"
import binascii

def _b(s):
    if isinstance(s, str):
        return s.encode('ascii')
    return bytes(s)

def b64encode(s, altchars=None):
    encoded = binascii.b2a_base64(s, newline=False)
    if altchars is not None:
        encoded = encoded.replace(b'+', altchars[0:1]).replace(b'/', altchars[1:2])
    return encoded

def b64decode(s, altchars=None, validate=False):
    s = _b(s)
    if altchars is not None:
        altchars = _b(altchars)
        s = s.replace(altchars[0:1], b'+').replace(altchars[1:2], b'/')
    return binascii.a2b_base64(s)

def standard_b64encode(s):
    return b64encode(s)

def standard_b64decode(s):
    return b64decode(s)

def urlsafe_b64encode(s):
    return b64encode(s).replace(b'+', b'-').replace(b'/', b'_')

def urlsafe_b64decode(s):
    s = _b(s).replace(b'-', b'+').replace(b'_', b'/')
    return b64decode(s)

def b16encode(s):
    return binascii.hexlify(s).upper()

def b16decode(s, casefold=False):
    return binascii.unhexlify(_b(s))

encodebytes = lambda s: binascii.b2a_base64(s)
decodebytes = lambda s: binascii.a2b_base64(s)
encodestring = encodebytes
decodestring = decodebytes
"##;

const DATETIME: &str = r##"
import time as _time
import math as _math

MINYEAR = 1
MAXYEAR = 9999
_DAYS_IN_MONTH = [-1, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
_DAYS_BEFORE_MONTH = [-1]
_dbm = 0
for _dim in _DAYS_IN_MONTH[1:]:
    _DAYS_BEFORE_MONTH.append(_dbm)
    _dbm += _dim
del _dbm, _dim

def _is_leap(year):
    return year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)

def _days_before_year(year):
    y = year - 1
    return y * 365 + y // 4 - y // 100 + y // 400

def _days_in_month(year, month):
    if month == 2 and _is_leap(year):
        return 29
    return _DAYS_IN_MONTH[month]

def _days_before_month(year, month):
    return _DAYS_BEFORE_MONTH[month] + (month > 2 and _is_leap(year))

def _ymd2ord(year, month, day):
    return _days_before_year(year) + _days_before_month(year, month) + day

_DI400Y = _days_before_year(401)
_DI100Y = _days_before_year(101)
_DI4Y = _days_before_year(5)

def _ord2ymd(n):
    n -= 1
    n400, n = divmod(n, _DI400Y)
    year = n400 * 400 + 1
    n100, n = divmod(n, _DI100Y)
    n4, n = divmod(n, _DI4Y)
    n1, n = divmod(n, 365)
    year += n100 * 100 + n4 * 4 + n1
    if n1 == 4 or n100 == 4:
        return year - 1, 12, 31
    leapyear = n1 == 3 and (n4 != 24 or n100 == 3)
    month = (n + 50) >> 5
    preceding = _DAYS_BEFORE_MONTH[month] + (month > 2 and leapyear)
    if preceding > n:
        month -= 1
        preceding -= _DAYS_IN_MONTH[month] + (month == 2 and leapyear)
    n -= preceding
    return year, month, n + 1

_MONTHNAMES = [None, "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
_DAYNAMES = [None, "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

class timedelta(object):
    def __init__(self, days=0, seconds=0, microseconds=0, milliseconds=0, minutes=0, hours=0, weeks=0):
        total = (((weeks * 7 + days) * 24 + hours) * 3600 + minutes * 60 + seconds) * 1000000 + milliseconds * 1000 + microseconds
        total = int(round(total))
        d, rem = divmod(total, 86400 * 1000000)
        s, us = divmod(rem, 1000000)
        self._days = d
        self._seconds = s
        self._microseconds = us
    days = property(lambda self: self._days)
    seconds = property(lambda self: self._seconds)
    microseconds = property(lambda self: self._microseconds)
    def total_seconds(self):
        return ((self._days * 86400 + self._seconds) * 10**6 + self._microseconds) / 10**6
    def _us(self):
        return (self._days * 86400 + self._seconds) * 1000000 + self._microseconds
    def __add__(self, other):
        if isinstance(other, timedelta):
            return timedelta(microseconds=self._us() + other._us())
        return NotImplemented
    __radd__ = __add__
    def __sub__(self, other):
        if isinstance(other, timedelta):
            return timedelta(microseconds=self._us() - other._us())
        return NotImplemented
    def __neg__(self):
        return timedelta(microseconds=-self._us())
    def __mul__(self, other):
        return timedelta(microseconds=self._us() * other)
    __rmul__ = __mul__
    def __truediv__(self, other):
        if isinstance(other, timedelta):
            return self._us() / other._us()
        return timedelta(microseconds=self._us() / other)
    def __floordiv__(self, other):
        if isinstance(other, timedelta):
            return self._us() // other._us()
        return timedelta(microseconds=self._us() // other)
    def __eq__(self, other):
        return isinstance(other, timedelta) and self._us() == other._us()
    def __lt__(self, other):
        return self._us() < other._us()
    def __le__(self, other):
        return self._us() <= other._us()
    def __gt__(self, other):
        return self._us() > other._us()
    def __ge__(self, other):
        return self._us() >= other._us()
    def __hash__(self):
        return hash(self._us())
    def __bool__(self):
        return self._us() != 0
    def __abs__(self):
        return -self if self._us() < 0 else self
    def __repr__(self):
        args = []
        if self._days:
            args.append("days=%d" % self._days)
        if self._seconds:
            args.append("seconds=%d" % self._seconds)
        if self._microseconds:
            args.append("microseconds=%d" % self._microseconds)
        return "datetime.timedelta(%s)" % ", ".join(args or ["0"])
    def __str__(self):
        mm, ss = divmod(self._seconds, 60)
        hh, mm = divmod(mm, 60)
        s = "%d:%02d:%02d" % (hh, mm, ss)
        if self._days:
            s = ("%d day%s, " % (self._days, "s" if abs(self._days) != 1 else "")) + s
        if self._microseconds:
            s += ".%06d" % self._microseconds
        return s
    def __reduce__(self):
        return (timedelta, (self._days, self._seconds, self._microseconds))

timedelta.min = timedelta(-999999999)
timedelta.max = timedelta(days=999999999, hours=23, minutes=59, seconds=59, microseconds=999999)
timedelta.resolution = timedelta(microseconds=1)

class tzinfo(object):
    def utcoffset(self, dt):
        return None
    def dst(self, dt):
        return None
    def tzname(self, dt):
        return None

class timezone(tzinfo):
    def __init__(self, offset, name=None):
        self._offset = offset
        self._name = name
    def utcoffset(self, dt):
        return self._offset
    def tzname(self, dt):
        return self._name or 'UTC'
    def dst(self, dt):
        return None
timezone.utc = timezone(timedelta(0))

class date(object):
    def __init__(self, year, month=None, day=None):
        if not 1 <= month <= 12:
            raise ValueError('month must be in 1..12')
        if not 1 <= day <= _days_in_month(year, month):
            raise ValueError('day is out of range for month')
        self._year = year
        self._month = month
        self._day = day
    year = property(lambda self: self._year)
    month = property(lambda self: self._month)
    day = property(lambda self: self._day)
    @classmethod
    def today(cls):
        t = _time.localtime()
        return cls(t[0], t[1], t[2])
    @classmethod
    def fromordinal(cls, n):
        y, m, d = _ord2ymd(n)
        return cls(y, m, d)
    @classmethod
    def fromtimestamp(cls, t):
        y, m, d = _time.localtime(t)[:3]
        return cls(y, m, d)
    @classmethod
    def fromisoformat(cls, s):
        return cls(int(s[0:4]), int(s[5:7]), int(s[8:10]))
    def toordinal(self):
        return _ymd2ord(self._year, self._month, self._day)
    def weekday(self):
        return (self.toordinal() + 6) % 7
    def isoweekday(self):
        return self.weekday() + 1
    def isoformat(self):
        return "%04d-%02d-%02d" % (self._year, self._month, self._day)
    __str__ = isoformat
    def timetuple(self):
        return _time.struct_time((self._year, self._month, self._day, 0, 0, 0, self.weekday(), self.toordinal() - _ymd2ord(self._year, 1, 1) + 1, -1)) if False else (self._year, self._month, self._day, 0, 0, 0, self.weekday(), self.toordinal() - _ymd2ord(self._year, 1, 1) + 1, -1)
    def strftime(self, fmt):
        return _time.strftime(fmt, self.timetuple())
    def __format__(self, fmt):
        return self.strftime(fmt) if fmt else str(self)
    def replace(self, year=None, month=None, day=None):
        return type(self)(year or self._year, month or self._month, day or self._day)
    def ctime(self):
        return "%s %s %2d 00:00:00 %04d" % (_DAYNAMES[self.isoweekday()], _MONTHNAMES[self._month], self._day, self._year)
    def __repr__(self):
        return "datetime.date(%d, %d, %d)" % (self._year, self._month, self._day)
    def _cmpkey(self):
        return (self._year, self._month, self._day)
    def __eq__(self, other):
        return isinstance(other, date) and not isinstance(other, datetime) and self._cmpkey() == other._cmpkey()
    def __lt__(self, other):
        return self._cmpkey() < other._cmpkey()
    def __le__(self, other):
        return self._cmpkey() <= other._cmpkey()
    def __gt__(self, other):
        return self._cmpkey() > other._cmpkey()
    def __ge__(self, other):
        return self._cmpkey() >= other._cmpkey()
    def __hash__(self):
        return hash(self._cmpkey())
    def __add__(self, other):
        if isinstance(other, timedelta):
            return type(self).fromordinal(self.toordinal() + other.days)
        return NotImplemented
    __radd__ = __add__
    def __sub__(self, other):
        if isinstance(other, timedelta):
            return type(self).fromordinal(self.toordinal() - other.days)
        if isinstance(other, date):
            return timedelta(days=self.toordinal() - other.toordinal())
        return NotImplemented
    def __reduce__(self):
        return (type(self), (self._year, self._month, self._day))

class time(object):
    def __init__(self, hour=0, minute=0, second=0, microsecond=0, tzinfo=None):
        self._hour = hour
        self._minute = minute
        self._second = second
        self._microsecond = microsecond
        self._tzinfo = tzinfo
    hour = property(lambda self: self._hour)
    minute = property(lambda self: self._minute)
    second = property(lambda self: self._second)
    microsecond = property(lambda self: self._microsecond)
    tzinfo = property(lambda self: self._tzinfo)
    def isoformat(self):
        s = "%02d:%02d:%02d" % (self._hour, self._minute, self._second)
        if self._microsecond:
            s += ".%06d" % self._microsecond
        return s
    __str__ = isoformat
    def strftime(self, fmt):
        return _time.strftime(fmt, (1900, 1, 1, self._hour, self._minute, self._second, 0, 1, -1))
    def __repr__(self):
        return "datetime.time(%d, %d, %d)" % (self._hour, self._minute, self._second)
    def _cmpkey(self):
        return (self._hour, self._minute, self._second, self._microsecond)
    def __eq__(self, other):
        return isinstance(other, time) and self._cmpkey() == other._cmpkey()
    def __lt__(self, other):
        return self._cmpkey() < other._cmpkey()
    def __hash__(self):
        return hash(self._cmpkey())

class datetime(date):
    def __init__(self, year, month=None, day=None, hour=0, minute=0, second=0, microsecond=0, tzinfo=None):
        date.__init__(self, year, month, day)
        self._hour = hour
        self._minute = minute
        self._second = second
        self._microsecond = microsecond
        self._tzinfo = tzinfo
    hour = property(lambda self: self._hour)
    minute = property(lambda self: self._minute)
    second = property(lambda self: self._second)
    microsecond = property(lambda self: self._microsecond)
    tzinfo = property(lambda self: self._tzinfo)
    @classmethod
    def fromtimestamp(cls, t, tz=None):
        frac, t = _math.modf(t)
        us = round(frac * 1e6)
        if us >= 1000000:
            t += 1
            us -= 1000000
        elif us < 0:
            t -= 1
            us += 1000000
        y, m, d, hh, mm, ss = (_time.gmtime(t) if tz is not None else _time.localtime(t))[:6]
        return cls(y, m, d, hh, mm, ss, us, tz)
    @classmethod
    def utcfromtimestamp(cls, t):
        y, m, d, hh, mm, ss = _time.gmtime(t)[:6]
        return cls(y, m, d, hh, mm, ss)
    @classmethod
    def now(cls, tz=None):
        return cls.fromtimestamp(_time.time(), tz)
    @classmethod
    def today(cls):
        return cls.now()
    @classmethod
    def utcnow(cls):
        return cls.utcfromtimestamp(_time.time())
    @classmethod
    def combine(cls, d, t, tzinfo=True):
        return cls(d.year, d.month, d.day, t.hour, t.minute, t.second, t.microsecond)
    @classmethod
    def strptime(cls, s, fmt):
        raise ValueError("strptime is not supported")
    @classmethod
    def fromisoformat(cls, s):
        d = date.fromisoformat(s[:10])
        hh = mm = ss = 0
        if len(s) > 11:
            hh = int(s[11:13])
            mm = int(s[14:16])
            if len(s) > 17:
                ss = int(s[17:19])
        return cls(d.year, d.month, d.day, hh, mm, ss)
    def date(self):
        return date(self._year, self._month, self._day)
    def time(self):
        return time(self._hour, self._minute, self._second, self._microsecond)
    def timestamp(self):
        return _time.mktime(self.timetuple()) + self._microsecond / 1e6
    def timetuple(self):
        return (self._year, self._month, self._day, self._hour, self._minute, self._second, self.weekday(), self.toordinal() - _ymd2ord(self._year, 1, 1) + 1, -1)
    def isoformat(self, sep='T'):
        s = "%04d-%02d-%02d%s%02d:%02d:%02d" % (self._year, self._month, self._day, sep, self._hour, self._minute, self._second)
        if self._microsecond:
            s += ".%06d" % self._microsecond
        return s
    def __str__(self):
        return self.isoformat(' ')
    def replace(self, year=None, month=None, day=None, hour=None, minute=None, second=None, microsecond=None, tzinfo=True):
        return type(self)(year if year is not None else self._year, month if month is not None else self._month, day if day is not None else self._day,
                          hour if hour is not None else self._hour, minute if minute is not None else self._minute,
                          second if second is not None else self._second, microsecond if microsecond is not None else self._microsecond)
    def ctime(self):
        return "%s %s %2d %02d:%02d:%02d %04d" % (_DAYNAMES[self.isoweekday()], _MONTHNAMES[self._month], self._day, self._hour, self._minute, self._second, self._year)
    def __repr__(self):
        return "datetime.datetime(%d, %d, %d, %d, %d, %d%s)" % (self._year, self._month, self._day, self._hour, self._minute, self._second, (", %d" % self._microsecond) if self._microsecond else "")
    def _cmpkey(self):
        return (self._year, self._month, self._day, self._hour, self._minute, self._second, self._microsecond)
    def __eq__(self, other):
        return isinstance(other, datetime) and self._cmpkey() == other._cmpkey()
    def __hash__(self):
        return hash(self._cmpkey())
    def _total_us(self):
        return ((self.toordinal() * 86400 + self._hour * 3600 + self._minute * 60 + self._second) * 1000000 + self._microsecond)
    def __add__(self, other):
        if isinstance(other, timedelta):
            us = self._total_us() + other._us()
            days, rem = divmod(us, 86400 * 1000000)
            s, usec = divmod(rem, 1000000)
            y, m, d = _ord2ymd(days)
            hh, rem2 = divmod(s, 3600)
            mm, ss = divmod(rem2, 60)
            return type(self)(y, m, d, hh, mm, ss, usec, self._tzinfo)
        return NotImplemented
    __radd__ = __add__
    def __sub__(self, other):
        if isinstance(other, timedelta):
            return self + (-other)
        if isinstance(other, datetime):
            return timedelta(microseconds=self._total_us() - other._total_us())
        return NotImplemented
    def utcoffset(self):
        return None if self._tzinfo is None else self._tzinfo.utcoffset(self)
    def astimezone(self, tz=None):
        return self
    def __reduce__(self):
        return (type(self), (self._year, self._month, self._day, self._hour, self._minute, self._second, self._microsecond))

date.min = date(1, 1, 1)
date.max = date(9999, 12, 31)
datetime.min = datetime(1, 1, 1)
datetime.max = datetime(9999, 12, 31, 23, 59, 59, 999999)
UTC = timezone.utc
"##;

const COLORSYS: &str = r##"
ONE_THIRD = 1.0 / 3.0
ONE_SIXTH = 1.0 / 6.0
TWO_THIRD = 2.0 / 3.0

def rgb_to_yiq(r, g, b):
    y = 0.30 * r + 0.59 * g + 0.11 * b
    i = 0.74 * (r - y) - 0.27 * (b - y)
    q = 0.48 * (r - y) + 0.41 * (b - y)
    return (y, i, q)

def yiq_to_rgb(y, i, q):
    r = y + 0.9468822170900693 * i + 0.6235565819861433 * q
    g = y - 0.27478764629897834 * i - 0.6356910791873801 * q
    b = y - 1.1085450346420322 * i + 1.7090069284064666 * q
    r = min(max(r, 0.0), 1.0)
    g = min(max(g, 0.0), 1.0)
    b = min(max(b, 0.0), 1.0)
    return (r, g, b)

def rgb_to_hls(r, g, b):
    maxc = max(r, g, b)
    minc = min(r, g, b)
    sumc = (maxc + minc)
    rangec = (maxc - minc)
    l = sumc / 2.0
    if minc == maxc:
        return 0.0, l, 0.0
    if l <= 0.5:
        s = rangec / sumc
    else:
        s = rangec / (2.0 - maxc - minc)
    rc = (maxc - r) / rangec
    gc = (maxc - g) / rangec
    bc = (maxc - b) / rangec
    if r == maxc:
        h = bc - gc
    elif g == maxc:
        h = 2.0 + rc - bc
    else:
        h = 4.0 + gc - rc
    h = (h / 6.0) % 1.0
    return h, l, s

def hls_to_rgb(h, l, s):
    if s == 0.0:
        return l, l, l
    if l <= 0.5:
        m2 = l * (1.0 + s)
    else:
        m2 = l + s - (l * s)
    m1 = 2.0 * l - m2
    return (_v(m1, m2, h + ONE_THIRD), _v(m1, m2, h), _v(m1, m2, h - ONE_THIRD))

def _v(m1, m2, hue):
    hue = hue % 1.0
    if hue < ONE_SIXTH:
        return m1 + (m2 - m1) * hue * 6.0
    if hue < 0.5:
        return m2
    if hue < TWO_THIRD:
        return m1 + (m2 - m1) * (TWO_THIRD - hue) * 6.0
    return m1

def rgb_to_hsv(r, g, b):
    maxc = max(r, g, b)
    minc = min(r, g, b)
    rangec = (maxc - minc)
    v = maxc
    if minc == maxc:
        return 0.0, 0.0, v
    s = rangec / maxc
    rc = (maxc - r) / rangec
    gc = (maxc - g) / rangec
    bc = (maxc - b) / rangec
    if r == maxc:
        h = bc - gc
    elif g == maxc:
        h = 2.0 + rc - bc
    else:
        h = 4.0 + gc - rc
    h = (h / 6.0) % 1.0
    return h, s, v

def hsv_to_rgb(h, s, v):
    if s == 0.0:
        return v, v, v
    i = int(h * 6.0)
    f = (h * 6.0) - i
    p = v * (1.0 - s)
    q = v * (1.0 - s * f)
    t = v * (1.0 - s * (1.0 - f))
    i = i % 6
    if i == 0:
        return v, t, p
    if i == 1:
        return q, v, p
    if i == 2:
        return p, v, t
    if i == 3:
        return p, q, v
    if i == 4:
        return t, p, v
    if i == 5:
        return v, p, q
"##;

const TYPING: &str = r##"
class _Special:
    def __init__(self, name):
        self._name = name
    def __getitem__(self, item):
        return self
    def __call__(self, *args, **kwargs):
        return args[0] if args else None
    def __repr__(self):
        return 'typing.' + self._name
    def __or__(self, other):
        return self
    def __ror__(self, other):
        return self
    def __instancecheck__(self, obj):
        return True
for _n in ['Any', 'Union', 'Optional', 'List', 'Dict', 'Tuple', 'Set', 'FrozenSet', 'Callable', 'Type', 'Iterable',
           'Iterator', 'Sequence', 'Mapping', 'MutableMapping', 'Generator', 'ClassVar', 'Final', 'Literal', 'Annotated',
           'NoReturn', 'Never', 'Self', 'TypeAlias', 'Hashable', 'Sized', 'Collection', 'Container', 'Awaitable', 'Coroutine',
           'AnyStr', 'Text', 'Deque', 'DefaultDict', 'OrderedDict', 'Counter', 'ChainMap', 'IO', 'TextIO', 'BinaryIO',
           'Pattern', 'Match', 'SupportsInt', 'SupportsFloat', 'Reversible', 'MutableSequence', 'MutableSet', 'AbstractSet', 'Concatenate', 'ParamSpec', 'Required', 'NotRequired', 'Unpack', 'TypeGuard', 'LiteralString']:
    globals()[_n] = _Special(_n)
TYPE_CHECKING = False
def TypeVar(name, *constraints, **kwargs):
    return _Special(name)
def TypeVarTuple(name, **kwargs):
    return _Special(name)
class TypeAliasType:
    """The value of a `type` statement; `__value__` is evaluated on use."""
    def __init__(self, name, value, *, type_params=()):
        self.__name__ = name
        self._evaluate = value if callable(value) and getattr(value, '__name__', '') == '<lambda>' else (lambda: value)
        self.__type_params__ = tuple(type_params)
    @property
    def __value__(self):
        return self._evaluate()
    def __getitem__(self, item):
        return self
    def __or__(self, other):
        return Union
    def __ror__(self, other):
        return Union
    def __repr__(self):
        return self.__name__
def NewType(name, tp):
    return lambda x: x
def cast(typ, val):
    return val
def overload(func):
    return func
def no_type_check(func):
    return func
def final(f):
    return f
def runtime_checkable(cls):
    return cls
def get_type_hints(obj, globalns=None, localns=None):
    return getattr(obj, '__annotations__', {})
class Generic:
    def __class_getitem__(cls, params):
        return cls
class Protocol(Generic):
    pass
def NamedTuple(typename, fields=None, **kwargs):
    from collections import namedtuple
    if fields is None:
        fields = kwargs.items()
    return namedtuple(typename, [f[0] for f in fields])
class TypedDict(dict):
    pass
"##;

const NUMBERS: &str = r##"
from abc import ABCMeta
class Number(metaclass=ABCMeta):
    __slots__ = ()
class Complex(Number):
    __slots__ = ()
class Real(Complex):
    __slots__ = ()
class Rational(Real):
    __slots__ = ()
class Integral(Rational):
    __slots__ = ()
Complex.register(complex)
Real.register(float)
Integral.register(int)
Number.register(int)
Number.register(float)
Number.register(complex)
Complex.register(int)
Complex.register(float)
Real.register(int)
Rational.register(int)
"##;

const CODECS: &str = r##"
BOM_UTF8 = b'\xef\xbb\xbf'
BOM_UTF16_LE = b'\xff\xfe'
BOM_UTF16_BE = b'\xfe\xff'
BOM_LE = BOM_UTF16_LE
BOM_BE = BOM_UTF16_BE
BOM = BOM_UTF16_LE
BOM_UTF16 = BOM_UTF16_LE
_errors = {}
def encode(obj, encoding='utf-8', errors='strict'):
    return obj.encode(encoding, errors)
def decode(obj, encoding='utf-8', errors='strict'):
    return obj.decode(encoding, errors)
def open(filename, mode='r', encoding=None, errors='strict', buffering=-1):
    import io
    if 'b' not in mode and encoding is None:
        encoding = 'utf-8'
    return io.open(filename, mode, encoding=encoding, errors=errors) if 'b' not in mode else io.open(filename, mode)
class CodecInfo(tuple):
    def __new__(cls, name):
        self = tuple.__new__(cls, (None, None, None, None))
        self.name = name
        return self
def lookup(encoding):
    "".encode(encoding)
    return CodecInfo(encoding.lower().replace('_', '-'))
def register_error(name, handler):
    _errors[name] = handler
def lookup_error(name):
    return _errors.get(name, lambda e: None)
def register(search_function):
    pass
def getencoder(encoding):
    return lambda s, errors='strict': (s.encode(encoding, errors), len(s))
def getdecoder(encoding):
    return lambda b, errors='strict': (b.decode(encoding, errors), len(b))
def utf_8_decode(b, errors='strict', final=False):
    return b.decode('utf-8', errors), len(b)
def utf_8_encode(s, errors='strict'):
    return s.encode('utf-8', errors), len(s)
class StreamReader(object):
    pass
class StreamWriter(object):
    pass
class IncrementalDecoder(object):
    def __init__(self, errors='strict'):
        self.errors = errors
"##;

const UNICODEDATA: &str = r##"
unidata_version = '13.0.0'
def category(c):
    if c.isupper():
        return 'Lu'
    if c.islower():
        return 'Ll'
    if c.isdigit():
        return 'Nd'
    if c.isspace():
        return 'Zs'
    if c.isalpha():
        return 'Lo'
    if ord(c) < 32:
        return 'Cc'
    return 'Po'
def east_asian_width(c):
    o = ord(c)
    if 0x1100 <= o <= 0x115f or 0x2e80 <= o <= 0xa4cf or 0xac00 <= o <= 0xd7a3 or 0xf900 <= o <= 0xfaff or 0xfe30 <= o <= 0xfe4f or 0xff00 <= o <= 0xff60 or 0xffe0 <= o <= 0xffe6 or 0x20000 <= o <= 0x3fffd:
        return 'W'
    if 0xff61 <= o <= 0xffdc:
        return 'H'
    return 'Na' if o < 0x80 else 'N'
def normalize(form, s):
    return s
def is_normalized(form, s):
    return True
def combining(c):
    return 0
def decimal(c, default=None):
    if c.isdigit():
        return int(c)
    if default is None:
        raise ValueError('not a decimal')
    return default
digit = decimal
def numeric(c, default=None):
    return decimal(c, default)
def name(c, default=None):
    if default is None:
        raise ValueError('no such name')
    return default
def lookup(n):
    raise KeyError("undefined character name '%s'" % n)
def mirrored(c):
    return 0
def bidirectional(c):
    return 'L'
"##;

const LOCALE: &str = r##"
LC_ALL = 6
LC_CTYPE = 0
LC_NUMERIC = 1
LC_TIME = 2
LC_COLLATE = 3
LC_MONETARY = 4
LC_MESSAGES = 5
class Error(Exception):
    pass
def setlocale(category, locale=None):
    return 'C'
def getlocale(category=0):
    return (None, None)
def getdefaultlocale(envvars=None):
    return ('en_US', 'UTF-8')
def getpreferredencoding(do_setlocale=True):
    return 'UTF-8'
def localeconv():
    return {'decimal_point': '.', 'thousands_sep': '', 'grouping': []}
def format_string(f, val, grouping=False, monetary=False):
    return f % val
def strcoll(a, b):
    return (a > b) - (a < b)
def strxfrm(s):
    return s
"##;

const PLATFORM: &str = r##"
def system():
    return 'CrySirror'
def machine():
    return ''
def node():
    return ''
def release():
    return ''
def version():
    return ''
def platform(aliased=False, terse=False):
    return 'CrySirror'
def python_version():
    import sys
    return '%d.%d.%d' % sys.version_info[:3]
def python_implementation():
    return 'CPython'
def architecture(executable=None, bits='', linkage=''):
    return ('64bit', '')
def uname():
    return ('CrySirror', '', '', '', '', '')
def win32_ver(*a):
    return ('', '', '', '')
def mac_ver(*a):
    return ('', ('', '', ''), '')
def libc_ver(*a):
    return ('', '')
"##;

const FNMATCH: &str = r##"
import re
def translate(pat):
    i, n = 0, len(pat)
    res = ''
    while i < n:
        c = pat[i]
        i = i + 1
        if c == '*':
            res = res + '.*'
        elif c == '?':
            res = res + '.'
        elif c == '[':
            j = i
            if j < n and pat[j] == '!':
                j = j + 1
            if j < n and pat[j] == ']':
                j = j + 1
            while j < n and pat[j] != ']':
                j = j + 1
            if j >= n:
                res = res + '\\['
            else:
                stuff = pat[i:j].replace('\\', '\\\\')
                i = j + 1
                if stuff[0] == '!':
                    stuff = '^' + stuff[1:]
                elif stuff[0] == '^':
                    stuff = '\\' + stuff
                res = '%s[%s]' % (res, stuff)
        else:
            res = res + re.escape(c)
    return '(?s:%s)\\Z' % res
def fnmatchcase(name, pat):
    return re.match(translate(pat), name) is not None
def fnmatch(name, pat):
    return fnmatchcase(name.lower(), pat.lower()) if False else fnmatchcase(name, pat)
def filter(names, pat):
    return [n for n in names if fnmatch(n, pat)]
"##;

const THREADING: &str = r##"
class Lock(object):
    def acquire(self, blocking=True, timeout=-1):
        return True
    def release(self):
        pass
    def locked(self):
        return False
    def __enter__(self):
        return True
    def __exit__(self, *a):
        pass
RLock = Lock
allocate_lock = Lock
LockType = Lock
class Condition(Lock):
    def __init__(self, lock=None):
        pass
    def wait(self, timeout=None):
        return True
    def notify(self, n=1):
        pass
    def notify_all(self):
        pass
    notifyAll = notify_all
class Event(object):
    def __init__(self):
        self._flag = False
    def is_set(self):
        return self._flag
    isSet = is_set
    def set(self):
        self._flag = True
    def clear(self):
        self._flag = False
    def wait(self, timeout=None):
        return self._flag
class Semaphore(Lock):
    def __init__(self, value=1):
        pass
BoundedSemaphore = Semaphore
class local(object):
    pass
class Thread(object):
    def __init__(self, group=None, target=None, name=None, args=(), kwargs=None, daemon=None):
        self._target = target
        self._args = args
        self._kwargs = kwargs or {}
        self.name = name or 'Thread'
        self.daemon = bool(daemon)
        self._started = False
    def start(self):
        # Scripts cannot create host threads; the target runs to completion.
        self._started = True
        self.run()
    def run(self):
        if self._target is not None:
            self._target(*self._args, **self._kwargs)
    def join(self, timeout=None):
        pass
    def is_alive(self):
        return False
    isAlive = is_alive
    def setDaemon(self, d):
        self.daemon = d
class Timer(Thread):
    def __init__(self, interval, function, args=None, kwargs=None):
        Thread.__init__(self, target=function, args=args or (), kwargs=kwargs)
    def cancel(self):
        pass
_main = Thread(name='MainThread')
def current_thread():
    return _main
currentThread = current_thread
def main_thread():
    return _main
def get_ident():
    return 1
def active_count():
    return 1
def enumerate():
    return [_main]
def start_new_thread(function, args, kwargs={}):
    function(*args, **kwargs)
    return 1
TIMEOUT_MAX = 1e9
"##;

const INSPECT: &str = r##"
import types as _types
def isclass(obj):
    return isinstance(obj, type)
def isfunction(obj):
    return isinstance(obj, _types.FunctionType)
def ismethod(obj):
    return isinstance(obj, _types.MethodType)
def ismodule(obj):
    return isinstance(obj, _types.ModuleType)
def isbuiltin(obj):
    return isinstance(obj, _types.BuiltinFunctionType)
def isroutine(obj):
    return isfunction(obj) or ismethod(obj) or isbuiltin(obj)
def isgenerator(obj):
    return isinstance(obj, _types.GeneratorType)
def isgeneratorfunction(obj):
    return isfunction(obj) and bool(obj.__code__.co_flags & 0x20)
def iscoroutinefunction(obj):
    return False
def isabstract(obj):
    return bool(getattr(obj, '__abstractmethods__', False))
def getmro(cls):
    return cls.__mro__
def getmembers(obj, predicate=None):
    results = []
    for key in dir(obj):
        try:
            value = getattr(obj, key)
        except AttributeError:
            continue
        if predicate is None or predicate(value):
            results.append((key, value))
    results.sort(key=lambda pair: pair[0])
    return results
def getdoc(obj):
    return getattr(obj, '__doc__', None)
def getmodule(obj):
    import sys
    return sys.modules.get(getattr(obj, '__module__', None))
def getfile(obj):
    return getattr(obj, '__file__', '<unknown>')
def getsourcefile(obj):
    return None
def getsource(obj):
    raise OSError('could not get source code')
def currentframe():
    import sys
    return sys._getframe(1)
def stack():
    return []
class Parameter(object):
    POSITIONAL_ONLY = 0
    POSITIONAL_OR_KEYWORD = 1
    VAR_POSITIONAL = 2
    KEYWORD_ONLY = 3
    VAR_KEYWORD = 4
    empty = object()
    def __init__(self, name, kind, default=None, annotation=None):
        self.name = name
        self.kind = kind
        self.default = Parameter.empty if default is None and kind != 1 else default
        self.annotation = annotation
class Signature(object):
    empty = Parameter.empty
    def __init__(self, parameters=None):
        self.parameters = dict((p.name, p) for p in (parameters or []))
def signature(func):
    code = getattr(func, '__code__', None)
    if code is None:
        f = getattr(func, '__func__', None) or getattr(getattr(func, '__init__', None), '__func__', None)
        code = getattr(f, '__code__', None)
    if code is None:
        return Signature([])
    names = code.co_varnames[:code.co_argcount]
    defaults = getattr(func, '__defaults__', None) or ()
    params = []
    nd = len(names) - len(defaults)
    for i, n in enumerate(names):
        if i >= nd:
            params.append(Parameter(n, Parameter.POSITIONAL_OR_KEYWORD, defaults[i - nd]))
        else:
            p = Parameter(n, Parameter.POSITIONAL_OR_KEYWORD)
            p.default = Parameter.empty
            params.append(p)
    return Signature(params)
class ArgSpec(tuple):
    pass
def getfullargspec(func):
    code = func.__code__
    args = list(code.co_varnames[:code.co_argcount])
    from collections import namedtuple
    FA = namedtuple('FullArgSpec', 'args varargs varkw defaults kwonlyargs kwonlydefaults annotations')
    return FA(args, None, None, func.__defaults__, [], None, {})
def getargspec(func):
    f = getfullargspec(func)
    return (f.args, f.varargs, f.varkw, f.defaults)
def unwrap(func, stop=None):
    while hasattr(func, '__wrapped__'):
        func = func.__wrapped__
    return func
"##;

const PPRINT: &str = r##"
def pformat(obj, indent=1, width=80, depth=None, compact=False, sort_dicts=True):
    if isinstance(obj, dict) and sort_dicts:
        try:
            items = sorted(obj.items())
        except TypeError:
            items = list(obj.items())
        return '{' + ', '.join('%r: %s' % (k, pformat(v)) for k, v in items) + '}'
    return repr(obj)
def pprint(obj, stream=None, indent=1, width=80, depth=None, compact=False, sort_dicts=True):
    import sys
    (stream or sys.stdout).write(pformat(obj, indent, width, depth, compact, sort_dicts) + '\n')
def saferepr(obj):
    return repr(obj)
def isreadable(obj):
    return True
def isrecursive(obj):
    return False
class PrettyPrinter(object):
    def __init__(self, indent=1, width=80, depth=None, stream=None, compact=False, sort_dicts=True):
        self._stream = stream
    def pformat(self, obj):
        return pformat(obj)
    def pprint(self, obj):
        pprint(obj, self._stream)
"##;

const REPRLIB: &str = r##"
import builtins
from itertools import islice
try:
    from _thread import get_ident
except ImportError:
    def get_ident():
        return 0

__all__ = ["Repr", "repr", "recursive_repr"]


def recursive_repr(fillvalue='...'):
    def decorating_function(user_function):
        repr_running = set()

        def wrapper(self):
            key = id(self), get_ident()
            if key in repr_running:
                return fillvalue
            repr_running.add(key)
            try:
                result = user_function(self)
            finally:
                repr_running.discard(key)
            return result

        for attr in ('__module__', '__doc__', '__name__', '__qualname__'):
            try:
                setattr(wrapper, attr, getattr(user_function, attr))
            except AttributeError:
                pass
        wrapper.__wrapped__ = user_function
        return wrapper
    return decorating_function


class Repr(object):

    def __init__(self, maxlevel=6, maxtuple=6, maxlist=6, maxarray=5, maxdict=4,
                 maxset=6, maxfrozenset=6, maxdeque=6, maxstring=30, maxlong=40,
                 maxother=30, fillvalue='...', indent=None):
        self.maxlevel = maxlevel
        self.maxtuple = maxtuple
        self.maxlist = maxlist
        self.maxarray = maxarray
        self.maxdict = maxdict
        self.maxset = maxset
        self.maxfrozenset = maxfrozenset
        self.maxdeque = maxdeque
        self.maxstring = maxstring
        self.maxlong = maxlong
        self.maxother = maxother
        self.fillvalue = fillvalue
        self.indent = indent

    def repr(self, x):
        return self.repr1(x, self.maxlevel)

    _lookup = {
        'tuple': 'builtins',
        'list': 'builtins',
        'array': 'array',
        'set': 'builtins',
        'frozenset': 'builtins',
        'deque': 'collections',
        'dict': 'builtins',
        'str': 'builtins',
        'int': 'builtins'
    }

    def repr1(self, x, level):
        cls = type(x)
        typename = cls.__name__
        if ' ' in typename:
            parts = typename.split()
            typename = '_'.join(parts)
        method = getattr(self, 'repr_' + typename, None)
        if method:
            # Not one of this class's types: a method of a subclass.
            if typename not in self._lookup:
                return method(x, level)
            module = getattr(cls, '__module__', None)
            # One of this class's types, from the module it is meant for.
            if module == self._lookup[typename]:
                return method(x, level)
        return self.repr_instance(x, level)

    def _join(self, pieces, level):
        if self.indent is None:
            return ', '.join(pieces)
        if not pieces:
            return ''
        indent = self.indent
        if isinstance(indent, int):
            indent = indent * ' '
        sep = ',\n' + (self.maxlevel - level + 1) * indent
        return sep.join(('',) + tuple(pieces) + ('',))[1:-len(indent) or None]

    def _repr_iterable(self, x, level, left, right, maxiter, trail=''):
        n = len(x)
        if level <= 0 and n:
            s = self.fillvalue
        else:
            newlevel = level - 1
            repr1 = self.repr1
            pieces = [repr1(elem, newlevel) for elem in islice(x, maxiter)]
            if n > maxiter:
                pieces.append(self.fillvalue)
            s = self._join(pieces, level)
            if n == 1 and trail and self.indent is None:
                right = trail + right
        return '%s%s%s' % (left, s, right)

    def repr_tuple(self, x, level):
        return self._repr_iterable(x, level, '(', ')', self.maxtuple, ',')

    def repr_list(self, x, level):
        return self._repr_iterable(x, level, '[', ']', self.maxlist)

    def repr_array(self, x, level):
        if not x:
            return "array('%s')" % x.typecode
        header = "array('%s', [" % x.typecode
        return self._repr_iterable(x, level, header, '])', self.maxarray)

    def repr_set(self, x, level):
        if not x:
            return 'set()'
        x = _possibly_sorted(x)
        return self._repr_iterable(x, level, '{', '}', self.maxset)

    def repr_frozenset(self, x, level):
        if not x:
            return 'frozenset()'
        x = _possibly_sorted(x)
        return self._repr_iterable(x, level, 'frozenset({', '})', self.maxfrozenset)

    def repr_deque(self, x, level):
        return self._repr_iterable(x, level, 'deque([', '])', self.maxdeque)

    def repr_dict(self, x, level):
        n = len(x)
        if n == 0:
            return '{}'
        if level <= 0:
            return '{' + self.fillvalue + '}'
        newlevel = level - 1
        repr1 = self.repr1
        pieces = []
        for key in islice(_possibly_sorted(x), self.maxdict):
            keyrepr = repr1(key, newlevel)
            valrepr = repr1(x[key], newlevel)
            pieces.append('%s: %s' % (keyrepr, valrepr))
        if n > self.maxdict:
            pieces.append(self.fillvalue)
        s = self._join(pieces, level)
        return '{%s}' % (s,)

    def repr_str(self, x, level):
        s = builtins.repr(x[:self.maxstring])
        if len(s) > self.maxstring:
            i = max(0, (self.maxstring - 3) // 2)
            j = max(0, self.maxstring - 3 - i)
            s = builtins.repr(x[:i] + x[len(x) - j:])
            s = s[:i] + self.fillvalue + s[len(s) - j:]
        return s

    repr_unicode = repr_str

    def repr_int(self, x, level):
        s = builtins.repr(x)
        if len(s) > self.maxlong:
            i = max(0, (self.maxlong - 3) // 2)
            j = max(0, self.maxlong - 3 - i)
            s = s[:i] + self.fillvalue + s[len(s) - j:]
        return s

    repr_long = repr_int

    def repr_instance(self, x, level):
        try:
            s = builtins.repr(x)
        except Exception:
            return '<%s instance at %#x>' % (x.__class__.__name__, id(x))
        if len(s) > self.maxother:
            i = max(0, (self.maxother - 3) // 2)
            j = max(0, self.maxother - 3 - i)
            s = s[:i] + self.fillvalue + s[len(s) - j:]
        return s


def _possibly_sorted(x):
    try:
        return sorted(x)
    except Exception:
        return list(x)


aRepr = Repr()
repr = aRepr.repr
"##;

const FRACTIONS: &str = r##"
import math
class Fraction(object):
    def __init__(self, numerator=0, denominator=None):
        if denominator is None:
            if isinstance(numerator, Fraction):
                n, d = numerator.numerator, numerator.denominator
            elif isinstance(numerator, float):
                n, d = numerator.as_integer_ratio()
            elif isinstance(numerator, str):
                if '/' in numerator:
                    a, b = numerator.split('/')
                    n, d = int(a), int(b)
                else:
                    n, d = Fraction(float(numerator)).numerator, Fraction(float(numerator)).denominator
            else:
                n, d = int(numerator), 1
        else:
            n, d = numerator, denominator
        if d == 0:
            raise ZeroDivisionError('Fraction(%s, 0)' % n)
        g = math.gcd(n, d)
        if d < 0:
            g = -g
        self.numerator = n // g
        self.denominator = d // g
    def _o(self, other):
        return other if isinstance(other, Fraction) else Fraction(other)
    def __add__(self, other):
        if isinstance(other, float):
            return float(self) + other
        o = self._o(other)
        return Fraction(self.numerator * o.denominator + o.numerator * self.denominator, self.denominator * o.denominator)
    __radd__ = __add__
    def __sub__(self, other):
        if isinstance(other, float):
            return float(self) - other
        o = self._o(other)
        return Fraction(self.numerator * o.denominator - o.numerator * self.denominator, self.denominator * o.denominator)
    def __rsub__(self, other):
        return self._o(other) - self
    def __mul__(self, other):
        if isinstance(other, float):
            return float(self) * other
        o = self._o(other)
        return Fraction(self.numerator * o.numerator, self.denominator * o.denominator)
    __rmul__ = __mul__
    def __truediv__(self, other):
        if isinstance(other, float):
            return float(self) / other
        o = self._o(other)
        return Fraction(self.numerator * o.denominator, self.denominator * o.numerator)
    def __rtruediv__(self, other):
        return self._o(other) / self
    def __neg__(self):
        return Fraction(-self.numerator, self.denominator)
    def __float__(self):
        return self.numerator / self.denominator
    def __int__(self):
        return int(self.numerator / self.denominator)
    def __eq__(self, other):
        if isinstance(other, float):
            return float(self) == other
        o = self._o(other)
        return self.numerator == o.numerator and self.denominator == o.denominator
    def __lt__(self, other):
        return float(self) < float(other)
    def __le__(self, other):
        return float(self) <= float(other)
    def __gt__(self, other):
        return float(self) > float(other)
    def __ge__(self, other):
        return float(self) >= float(other)
    def __hash__(self):
        return hash(float(self)) if self.denominator != 1 else hash(self.numerator)
    def __repr__(self):
        return 'Fraction(%d, %d)' % (self.numerator, self.denominator)
    def __str__(self):
        return str(self.numerator) if self.denominator == 1 else '%d/%d' % (self.numerator, self.denominator)
    def limit_denominator(self, max_denominator=1000000):
        return Fraction(round(float(self) * max_denominator), max_denominator)
"##;

const DATACLASSES: &str = r##"
MISSING = object()
class Field(object):
    def __init__(self, default=MISSING, default_factory=MISSING, init=True, repr=True, hash=None, compare=True, metadata=None, kw_only=False):
        self.default = default
        self.default_factory = default_factory
        self.init = init
        self.repr = repr
        self.compare = compare
        self.metadata = metadata
        self.name = None
def field(**kwargs):
    return Field(**kwargs)
def fields(obj):
    return tuple(getattr(obj, '__dataclass_fields__', {}).values())
def _process(cls, init, repr_, eq, order, frozen):
    ann = cls.__dict__.get('__annotations__', {})
    flds = {}
    for b in reversed(cls.__mro__[1:]):
        flds.update(getattr(b, '__dataclass_fields__', {}))
    for name in ann:
        default = cls.__dict__.get(name, MISSING)
        f = default if isinstance(default, Field) else Field(default=default)
        f.name = name
        flds[name] = f
        if isinstance(default, Field):
            if f.default is MISSING:
                try:
                    delattr(cls, name)
                except AttributeError:
                    pass
            else:
                setattr(cls, name, f.default)
    cls.__dataclass_fields__ = flds
    names = [f.name for f in flds.values() if f.init]
    if init:
        def __init__(self, *args, **kwargs):
            vals = list(args)
            for i, n in enumerate(names):
                f = flds[n]
                if i < len(vals):
                    v = vals[i]
                elif n in kwargs:
                    v = kwargs.pop(n)
                elif f.default is not MISSING:
                    v = f.default
                elif f.default_factory is not MISSING:
                    v = f.default_factory()
                else:
                    raise TypeError('__init__() missing required argument: %r' % n)
                object.__setattr__(self, n, v)
            for f in flds.values():
                if not f.init:
                    if f.default_factory is not MISSING:
                        object.__setattr__(self, f.name, f.default_factory())
                    elif f.default is not MISSING:
                        object.__setattr__(self, f.name, f.default)
            post = getattr(self, '__post_init__', None)
            if post is not None:
                post()
        cls.__init__ = __init__
    if repr_:
        def __repr__(self):
            return '%s(%s)' % (type(self).__name__, ', '.join('%s=%r' % (f.name, getattr(self, f.name)) for f in flds.values() if f.repr))
        cls.__repr__ = __repr__
    if eq:
        def __eq__(self, other):
            if other.__class__ is not self.__class__:
                return NotImplemented
            return all(getattr(self, f.name) == getattr(other, f.name) for f in flds.values() if f.compare)
        cls.__eq__ = __eq__
        cls.__hash__ = None if not frozen else (lambda self: hash(tuple(getattr(self, f.name) for f in flds.values())))
    if order:
        def _key(self):
            return tuple(getattr(self, f.name) for f in flds.values() if f.compare)
        cls.__lt__ = lambda a, b: _key(a) < _key(b)
        cls.__le__ = lambda a, b: _key(a) <= _key(b)
        cls.__gt__ = lambda a, b: _key(a) > _key(b)
        cls.__ge__ = lambda a, b: _key(a) >= _key(b)
    if frozen:
        def __setattr__(self, name, value):
            raise AttributeError("cannot assign to field %r" % name)
        cls.__setattr__ = __setattr__
    return cls
def dataclass(cls=None, *, init=True, repr=True, eq=True, order=False, unsafe_hash=False, frozen=False, **kwargs):
    def wrap(cls):
        return _process(cls, init, repr, eq, order, frozen)
    if cls is None:
        return wrap
    return wrap(cls)
def asdict(obj):
    return dict((f.name, getattr(obj, f.name)) for f in fields(obj))
def astuple(obj):
    return tuple(getattr(obj, f.name) for f in fields(obj))
def replace(obj, **changes):
    d = asdict(obj)
    d.update(changes)
    return type(obj)(**d)
def is_dataclass(obj):
    return hasattr(obj, '__dataclass_fields__')
class FrozenInstanceError(AttributeError):
    pass
"##;

const STATISTICS: &str = r##"
import math
def mean(data):
    data = list(data)
    if not data:
        raise ValueError('mean requires at least one data point')
    return sum(data) / len(data)
fmean = mean
def median(data):
    data = sorted(data)
    n = len(data)
    if n == 0:
        raise ValueError('no median for empty data')
    if n % 2 == 1:
        return data[n // 2]
    i = n // 2
    return (data[i - 1] + data[i]) / 2
def mode(data):
    data = list(data)
    return max(set(data), key=data.count)
def pvariance(data, mu=None):
    data = list(data)
    if mu is None:
        mu = mean(data)
    return sum((x - mu) ** 2 for x in data) / len(data)
def variance(data, xbar=None):
    data = list(data)
    if xbar is None:
        xbar = mean(data)
    return sum((x - xbar) ** 2 for x in data) / (len(data) - 1)
def pstdev(data, mu=None):
    return math.sqrt(pvariance(data, mu))
def stdev(data, xbar=None):
    return math.sqrt(variance(data, xbar))
"##;

const FUTURE_UTILS: &str = r##"
import sys
PY2 = sys.version_info[0] == 2
PY3 = not PY2
PY26 = False
PY27 = PY2
string_types = (str,)
text_type = str
binary_type = bytes
integer_types = (int,)
def bord(s):
    return s if isinstance(s, int) else ord(s)
def bchr(i):
    return bytes([i])
def tobytes(s):
    return s.encode('latin-1') if isinstance(s, str) else bytes(s)
def iteritems(d, **kw):
    return iter(d.items(**kw))
def itervalues(d, **kw):
    return iter(d.values(**kw))
def iterkeys(d, **kw):
    return iter(d.keys(**kw))
def with_metaclass(meta, *bases):
    class metaclass(meta):
        def __new__(cls, name, this_bases, d):
            return meta(name, bases, d)
    return type.__new__(metaclass, 'temporary_class', (), {})
def native_str_to_bytes(s, encoding='utf-8'):
    return s.encode(encoding)
def raise_from(exc, cause):
    exc.__cause__ = cause
    raise exc
def reraise(tp, value, tb=None):
    raise value
def python_2_unicode_compatible(cls):
    return cls
native_str = str
def isidentifier(s):
    return s.isidentifier()
def viewitems(d):
    return d.items()
def viewkeys(d):
    return d.keys()
def viewvalues(d):
    return d.values()
"##;

const SIX: &str = r##"
import sys
PY2 = sys.version_info[0] == 2
PY3 = not PY2
string_types = (str,)
text_type = str
binary_type = bytes
integer_types = (int,)
class_types = (type,)
MAXSIZE = sys.maxsize
def iteritems(d, **kw):
    return iter(d.items(**kw))
def itervalues(d, **kw):
    return iter(d.values(**kw))
def iterkeys(d, **kw):
    return iter(d.keys(**kw))
def with_metaclass(meta, *bases):
    class metaclass(meta):
        def __new__(cls, name, this_bases, d):
            return meta(name, bases, d)
    return type.__new__(metaclass, 'temporary_class', (), {})
def add_metaclass(metaclass):
    def wrapper(cls):
        orig_vars = cls.__dict__.copy()
        orig_vars.pop('__dict__', None)
        orig_vars.pop('__weakref__', None)
        return metaclass(cls.__name__, cls.__bases__, orig_vars)
    return wrapper
def b(s):
    return s.encode('latin-1')
def u(s):
    return s
def reraise(tp, value, tb=None):
    raise value
def raise_from(value, from_value):
    raise value
def print_(*args, **kwargs):
    print(*args, **kwargs)
def ensure_str(s, encoding='utf-8', errors='strict'):
    return s.decode(encoding, errors) if isinstance(s, bytes) else s
ensure_text = ensure_str
def ensure_binary(s, encoding='utf-8', errors='strict'):
    return s.encode(encoding, errors) if isinstance(s, str) else s
def python_2_unicode_compatible(klass):
    return klass
unichr = chr
next = next
callable = callable
from io import StringIO, BytesIO
moves = sys.modules[__name__]
"##;

const QUEUE: &str = r##"
from collections import deque
class Empty(Exception):
    pass
class Full(Exception):
    pass
class Queue(object):
    def __init__(self, maxsize=0):
        self.maxsize = maxsize
        self.queue = deque()
    def qsize(self):
        return len(self.queue)
    def empty(self):
        return not self.queue
    def full(self):
        return 0 < self.maxsize <= len(self.queue)
    def put(self, item, block=True, timeout=None):
        if self.full():
            raise Full
        self.queue.append(item)
    def put_nowait(self, item):
        self.put(item, False)
    def get(self, block=True, timeout=None):
        if not self.queue:
            raise Empty
        return self.queue.popleft()
    def get_nowait(self):
        return self.get(False)
    def task_done(self):
        pass
    def join(self):
        pass
class LifoQueue(Queue):
    def get(self, block=True, timeout=None):
        if not self.queue:
            raise Empty
        return self.queue.pop()
class PriorityQueue(Queue):
    def put(self, item, block=True, timeout=None):
        import heapq
        q = list(self.queue)
        heapq.heappush(q, item)
        self.queue = deque(q)
    def get(self, block=True, timeout=None):
        import heapq
        if not self.queue:
            raise Empty
        q = list(self.queue)
        item = heapq.heappop(q)
        self.queue = deque(q)
        return item
SimpleQueue = Queue
"##;

const CALENDAR: &str = r##"
import datetime as _dt
day_name = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']
day_abbr = [d[:3] for d in day_name]
month_name = ['', 'January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
month_abbr = [m[:3] for m in month_name]
MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY, SATURDAY, SUNDAY = range(7)
def isleap(year):
    return year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
def leapdays(y1, y2):
    y1 -= 1
    y2 -= 1
    return (y2 // 4 - y1 // 4) - (y2 // 100 - y1 // 100) + (y2 // 400 - y1 // 400)
def weekday(year, month, day):
    return _dt.date(year, month, day).weekday()
def monthrange(year, month):
    mdays = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    ndays = mdays[month] + (month == 2 and isleap(year))
    return weekday(year, month, 1), ndays
def timegm(tuple):
    year, month, day, hour, minute, second = tuple[:6]
    days = _dt.date(year, month, 1).toordinal() - _dt.date(1970, 1, 1).toordinal() + day - 1
    return ((days * 24 + hour) * 60 + minute) * 60 + second
def monthcalendar(year, month):
    first, n = monthrange(year, month)
    weeks = []
    week = [0] * first
    for d in range(1, n + 1):
        week.append(d)
        if len(week) == 7:
            weeks.append(week)
            week = []
    if week:
        weeks.append(week + [0] * (7 - len(week)))
    return weeks
"##;

const UUID: &str = r##"
import os
class UUID(object):
    def __init__(self, hex=None, bytes=None, int=None, version=None):
        if hex is not None:
            hex = hex.replace('urn:', '').replace('uuid:', '').strip('{}').replace('-', '')
            int = globals()['_int'](hex, 16)
        elif bytes is not None:
            int = globals()['_int'].from_bytes(bytes, 'big')
        self.int = int
    @property
    def hex(self):
        return '%032x' % self.int
    def __str__(self):
        h = self.hex
        return '%s-%s-%s-%s-%s' % (h[:8], h[8:12], h[12:16], h[16:20], h[20:])
    def __repr__(self):
        return 'UUID(%r)' % str(self)
    def __eq__(self, other):
        return isinstance(other, UUID) and self.int == other.int
    def __hash__(self):
        return hash(self.int)
    @property
    def bytes(self):
        return self.int.to_bytes(16, 'big')
_int = int
def uuid4():
    b = bytearray(os.urandom(16))
    b[6] = (b[6] & 0x0f) | 0x40
    b[8] = (b[8] & 0x3f) | 0x80
    return UUID(bytes=bytes(b))
def uuid1(node=None, clock_seq=None):
    return uuid4()
"##;

const ARRAY: &str = r##"
class array(list):
    def __init__(self, typecode, initializer=()):
        list.__init__(self, initializer)
        self.typecode = typecode
    def tolist(self):
        return list(self)
    def tobytes(self):
        import struct
        return struct.pack('<%d%s' % (len(self), self.typecode), *self)
    def frombytes(self, b):
        import struct
        n = len(b) // struct.calcsize(self.typecode)
        self.extend(struct.unpack('<%d%s' % (n, self.typecode), b))
    tostring = tobytes
    fromstring = frombytes
typecodes = 'bBuhHiIlLqQfd'
"##;
