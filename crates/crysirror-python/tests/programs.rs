// SPDX-License-Identifier: MPL-2.0
//! End-to-end programs: each script asserts its own results.

use crysirror_python::{run_source, Dialect, Vm};

fn run(src: &str) {
    let mut vm = Vm::new();
    if let Err(e) = run_source(&mut vm, src, Dialect::Py3) {
        panic!("{e}");
    }
}

fn run2(src: &str) {
    let mut vm = Vm::new();
    crysirror_python::modules::sys::set_dialect(&mut vm, Dialect::Py2);
    if let Err(e) = run_source(&mut vm, src, Dialect::Py2) {
        panic!("{e}");
    }
}

#[test]
fn arithmetic_and_strings() {
    run(r#"
assert 1 + 2 * 3 == 7
assert 7 // 2 == 3 and -7 // 2 == -4 and 7 % -3 == -2
assert 2 ** 100 == 1267650600228229401496703205376
assert 10 / 4 == 2.5
assert str(0.1 + 0.2) == '0.30000000000000004'
assert repr(1e16) == '1e+16'
assert "%5.2f|%-4d|%s|%r" % (3.14159, 42, 'x', 'y') == ' 3.14|42  |x|\'y\''
assert "{0}-{1:>4}-{name}".format(1, 'ab', name='z') == '1-  ab-z'
assert f"{3+4:03d} {'q'!r}" == "007 'q'"
s = "Hello, World"
assert s.lower().split(", ") == ['hello', 'world']
assert s[::-1] == 'dlroW ,olleH'
assert s.find('o') == 4 and s.rfind('o') == 8
assert ','.join(map(str, range(3))) == '0,1,2'
assert 'ab' * 3 == 'ababab'
assert len('日本語') == 3 and '日本語'[1] == '本'
"#);
}

#[test]
fn containers_and_comprehensions() {
    run(r#"
d = {'a': 1, 'b': 2}
d['c'] = 3
assert list(d) == ['a', 'b', 'c']
assert {k: v * 2 for k, v in d.items()} == {'a': 2, 'b': 4, 'c': 6}
assert [x * x for x in range(5) if x % 2] == [1, 9]
assert {x % 3 for x in range(10)} == {0, 1, 2}
assert sorted([3, 1, 2], reverse=True) == [3, 2, 1]
assert sorted(['b', 'A', 'c'], key=str.lower) == ['A', 'b', 'c']
a, *b, c = [1, 2, 3, 4]
assert (a, b, c) == (1, [2, 3], 4)
l = [1, 2, 3]
l[1:2] = [9, 9]
assert l == [1, 9, 9, 3]
del l[0]
assert l == [9, 9, 3]
assert sum(x for x in range(10)) == 45
assert max([1, 5, 2]) == 5 and min('bca') == 'a'
assert list(zip('ab', [1, 2])) == [('a', 1), ('b', 2)]
assert list(enumerate('xy', 1)) == [(1, 'x'), (2, 'y')]
assert dict(zip('ab', range(2))) == {'a': 0, 'b': 1}
s = {1, 2}
s |= {3}
assert s == {1, 2, 3}
assert (1, 2) < (1, 3) and [1, 2] == [1, 2]
"#);
}

#[test]
fn classes_closures_generators() {
    run(r#"
class Base(object):
    count = 0
    def __init__(self, x):
        self.x = x
        Base.count += 1
    def value(self):
        return self.x
    @property
    def double(self):
        return self.x * 2
    @classmethod
    def make(cls, x):
        return cls(x)
    @staticmethod
    def helper():
        return 'h'
    def __eq__(self, other):
        return isinstance(other, Base) and self.x == other.x
    def __hash__(self):
        return hash(self.x)
    def __repr__(self):
        return 'Base(%r)' % self.x

class Child(Base):
    def value(self):
        return super().value() + 1

c = Child.make(4)
assert c.value() == 5 and c.double == 8 and Base.helper() == 'h'
assert Base.count == 1 and isinstance(c, Base) and issubclass(Child, Base)
assert repr(c) == 'Base(4)' and Base(4) == c and len({Base(1), Base(1)}) == 1

def counter():
    n = 0
    def inc():
        nonlocal n
        n += 1
        return n
    return inc
f = counter()
f(); f()
assert f() == 3

def gen(n):
    for i in range(n):
        x = yield i
        if x:
            yield x * 10
g = gen(3)
assert next(g) == 0
assert g.send(5) == 50
assert list(gen(3)) == [0, 1, 2]

def deleg():
    r = yield from gen(2)
    yield 'done'
assert list(deleg()) == [0, 1, 'done']

class Meta(type):
    def __new__(mcls, name, bases, ns):
        ns['tag'] = name.lower()
        return super().__new__(mcls, name, bases, ns)
class T(metaclass=Meta):
    pass
assert T.tag == 't'

class Slotted:
    __private = 1
    def get(self):
        return self.__private
assert Slotted().get() == 1 and Slotted._Slotted__private == 1
"#);
}

#[test]
fn exceptions_and_with() {
    run(r#"
log = []
class CM:
    def __enter__(self):
        log.append('enter')
        return self
    def __exit__(self, t, v, tb):
        log.append(('exit', t.__name__ if t else None))
        return t is KeyError
with CM():
    raise KeyError('k')
with CM() as c:
    pass
assert log == ['enter', ('exit', 'KeyError'), 'enter', ('exit', None)]

def f():
    try:
        return 1
    finally:
        log.append('fin')
assert f() == 1 and log[-1] == 'fin'

try:
    try:
        1 / 0
    except ZeroDivisionError as e:
        raise ValueError('wrapped') from e
except ValueError as e:
    assert isinstance(e.__cause__, ZeroDivisionError)
    assert str(e) == 'wrapped'

def loop():
    out = []
    for i in range(5):
        try:
            if i == 3:
                break
            if i == 1:
                continue
            out.append(i)
        finally:
            out.append('f')
    return out
assert loop() == [0, 'f', 'f', 2, 'f', 'f']

try:
    {}['missing']
except KeyError as e:
    assert e.args == ('missing',)
else:
    assert False
try:
    raise
except RuntimeError:
    pass
"#);
}

#[test]
fn stdlib_modules() {
    run(r#"
import re, json, math, random, collections, functools, itertools, copy, string, operator
assert re.sub(r'(\w+)@(\w+)', r'\2 at \1', 'me@host') == 'host at me'
assert re.findall(r'\d+', 'a1b22c333') == ['1', '22', '333']
m = re.match(r'(?P<k>\w+)=(?P<v>\w+)', 'key=val')
assert m.group('k') == 'key' and m.groupdict() == {'k': 'key', 'v': 'val'}
assert re.split(r'\s*,\s*', 'a , b,c') == ['a', 'b', 'c']
assert json.loads(json.dumps({'a': [1, 2.5, None, True, 'x']})) == {'a': [1, 2.5, None, True, 'x']}
assert json.dumps({'b': 1, 'a': 2}, sort_keys=True) == '{"a": 2, "b": 1}'
assert math.isclose(math.sqrt(2) ** 2, 2) and math.floor(-1.5) == -2
random.seed(42)
assert random.random() == 0.6394267984578837
r = random.Random(1)
assert r.randint(1, 100) == 18
Point = collections.namedtuple('Point', 'x y')
p = Point(1, y=2)
assert p.x == 1 and p._replace(x=5) == (5, 2) and repr(p) == 'Point(x=1, y=2)'
od = collections.OrderedDict([('a', 1), ('b', 2)])
od.move_to_end('a')
assert list(od) == ['b', 'a']
dd = collections.defaultdict(list)
dd['x'].append(1)
assert dd == {'x': [1]}
dq = collections.deque([1, 2, 3], maxlen=3)
dq.append(4)
assert list(dq) == [2, 3, 4]
assert collections.Counter('abca').most_common(1) == [('a', 2)]
assert functools.reduce(operator.add, [1, 2, 3]) == 6
add3 = functools.partial(operator.add, 3)
assert add3(4) == 7
assert list(itertools.islice(itertools.count(10), 3)) == [10, 11, 12]
assert list(itertools.chain([1], [2, 3])) == [1, 2, 3]
assert list(itertools.product('ab', repeat=2)) == [('a', 'a'), ('a', 'b'), ('b', 'a'), ('b', 'b')]
x = {'l': [1, [2]]}
y = copy.deepcopy(x)
y['l'][1].append(3)
assert x == {'l': [1, [2]]} and y == {'l': [1, [2, 3]]}
assert string.ascii_lowercase[:3] == 'abc'
@functools.lru_cache(None)
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)
assert fib(80) == 23416728348467685
import enum
class Color(enum.Enum):
    RED = 1
    GREEN = 2
assert Color(2) is Color.GREEN and Color.RED.name == 'RED' and list(Color) == [Color.RED, Color.GREEN]
"#);
}

#[test]
fn pickle_roundtrip() {
    run(r#"
import pickle
class Obj(object):
    def __init__(self, a):
        self.a = a
data = {'x': [1, 2.5, 'three', (4, None)], 'o': Obj([1])}
data['self'] = data['x']
for proto in (0, 2, 4):
    out = pickle.loads(pickle.dumps(data, proto))
    assert out['x'] == data['x'] and out['self'] is out['x'] and out['o'].a == [1]
"#);
}

#[test]
fn python2_dialect() {
    run2(r#"
print "hello", 1
assert 7 / 2 == 3
assert 7.0 / 2 == 3.5
d = {'a': 1}
assert d.keys() == ['a'] and d.has_key('a')
assert range(3) == [0, 1, 2]
assert map(lambda x: x + 1, [1, 2]) == [2, 3]
try:
    raise ValueError, 'bad'
except ValueError, e:
    assert str(e) == 'bad'
x = 0777
assert x == 511 and `x` == '511' and 1 <> 2
assert None < 0 and 0 < 'a'
class Old:
    def __init__(self): self.v = 1
assert Old().v == 1
"#);
}

#[test]
fn python2_list_comprehensions_accept_legacy_tuple_iterables_only() {
    run2(r#"
assert [x * 2 for x in 0, 1, 2, if x] == [2, 4]
assert [x + y for x in 1, 2 for y in 3, 4] == [4, 5, 5, 6]
assert x == 2 and y == 4
"#);
    use crysirror_python::parser::{parse_expression, ParseOptions};
    assert!(parse_expression("[x for x in 0, 1]", ParseOptions::py3()).is_err());
    assert!(parse_expression("[x for x in 0,]", ParseOptions::py2()).is_err());
    assert!(parse_expression("(x for x in 0, 1)", ParseOptions::py2()).is_err());
}
