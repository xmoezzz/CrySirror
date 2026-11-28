import io
from renpy.error import (
    TracebackException, FrameSummary, NonColoredExceptionPrintContext,
    ANSIColoredPrintContext, TextIOExceptionPrintContext, compute_closest_value,
    handle_attribute_error, normalize_renpy_line_offset,
)

out = []


def fmt(te, **kw):
    return te.format(NonColoredExceptionPrintContext(io.StringIO(), **kw))


class Ctx(TextIOExceptionPrintContext):
    def __init__(self, *, filter_private, emit_context=True):
        super().__init__(io.StringIO(), filter_private=filter_private, max_group_width=5, max_group_depth=1, emit_context=emit_context)

    def location(self, filename, lineno, name):
        self._print(f'{{a=edit:{lineno}:{filename}}}File "{filename}", line {lineno}{{/a}}, in {name}')

    def source_carets(self, line, carets):
        self._print(line.replace("{", "{{") + ("" if carets is None else " | " + carets))

    def final_exception_line(self, exc_type, text):
        self._print(exc_type if text is None else f"{exc_type}: {text}")


te = TracebackException(ValueError("bad {value}"))
te.stack.extend([
    FrameSummary("script", "game/script.rpy", 10, text="    $ x = 1 + y"),
    FrameSummary("<module>", "renpy/common/00start.rpy", 5, text="foo()"),
    FrameSummary("f", "renpy/ast.py", 3, text="  bar  "),
    FrameSummary("<module>", "<string>", 1),
])
out.append(fmt(te))
out.append(fmt(te, filter_private=True, emit_context=False))
out.append(te.format(Ctx(filter_private=True, emit_context=False)))
out.append(te.format_exception_only())
out.append(str(te) + "|" + te.exc_type_str)

rec = TracebackException(RecursionError("deep"))
rec.stack.extend([FrameSummary("g", "game/r.rpy", 2, text="g()") for _ in range(7)])
rec.stack.append(FrameSummary("h", "game/r.rpy", 3, text="h()"))
out.append(fmt(rec))

a = KeyError("k")
b = RuntimeError("outer")
b.__cause__ = a
b.__suppress_context__ = True
out.append(fmt(TracebackException(b)))
c = RuntimeError("ctx")
c.__context__ = ValueError("inner")
out.append(fmt(TracebackException(c)))
out.append(fmt(TracebackException(c), chain=False) if False else TracebackException(c).format(chain=False))

s = SyntaxError("invalid syntax", ("game/a.rpy", 3, 9, "    x = = 1\n"))
out.append(fmt(TracebackException(s)))
s = SyntaxError("bad", ("game/b.rpy", 4, None, "  y\n"))
out.append(fmt(TracebackException(s)))
s = SyntaxError("far", ("game/c.rpy", 1, 40, "abc\n"))
out.append(fmt(TracebackException(s)))
out.append(fmt(TracebackException(SyntaxError(""))))
out.append(fmt(TracebackException(Exception())))

for col, end in ((4, 22), (4, 13), (16, 22), (4, 9)):
    f = FrameSummary("f", "game/t.rpy", 2, col, 2, end)
    t = TracebackException(Exception("x"))
    t.stack.append(f)
    out.append(fmt(t))
f = FrameSummary("f", "game/t.rpy", 3, 9, 5, 10)
t = TracebackException(Exception("multi"))
t.stack.append(f)
out.append(fmt(t))
f = FrameSummary("f", "game/t.rpy", 6, 4, 6, 9)
t = TracebackException(Exception("stmt"))
t.stack.append(f)
out.append(fmt(t))
f = FrameSummary("f", "game/t.rpy", 7, 4, 7, 17)
t = TracebackException(Exception("sub"))
t.stack.append(f)
out.append(fmt(t))
out.append(t.format(ANSIColoredPrintContext(io.StringIO())))
out.append(repr(list(f.carets)) + repr(list(f.significant_lines)))

out.append(repr([compute_closest_value(v, ["foo", "bar", "Foobar", "_x"]) for v in ("fo", "fooo", "FOO", "baz", "zzzzzz", "_y")]))


class A:
    apple = 1
    banana = 2
    _hidden = 3


e = AttributeError("no")
e.name = "appel"
e.obj = A()
out.append(repr(handle_attribute_error(e)))
e.name = "_hiden"
out.append(repr(handle_attribute_error(e)))
e.name = None
out.append(repr(handle_attribute_error(e)))
out.append(repr([normalize_renpy_line_offset("game/script.rpy", 1, n, "x = 'é' + __a") for n in (0, 5, 8, 12)]))
out.append(repr(normalize_renpy_line_offset("game/script.rpy", 1, 20, "x = _m1_script__a + _m1_script__b")))

RESULT = "\n=====\n".join(out)
