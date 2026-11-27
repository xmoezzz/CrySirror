# SPDX-License-Identifier: MPL-2.0
# Module globals and control exceptions of Ren'Py's renpy/game.py (MIT).
"""Engine-wide state. Contexts and nested execution are owned by the
CrySirror executor; `context()` returns the live execution context."""
import _renpy

basepath = None
searchpath = []
args = None
script = None
contexts = []
interface = None
lint = False
log = None
exception_info = ''
style = None
seen_session = {}
seen_translates_count = 0
new_translates_count = 0
after_rollback = False
post_init = []
less_memory = False
less_updates = False
less_mouse = False
less_imagedissolve = False
persistent = None
preferences = None
initcode_ast_id = 0
build_info = {"info": {}}


class ExceptionInfo(object):
    def __init__(self, s, args):
        self.s = s
        self.args = args

    def __enter__(self):
        return

    def __exit__(self, exc_type, exc_val, exc_tb):
        global exception_info
        if exc_type:
            exception_info = self.s % self.args
        return False


class RestartContext(Exception):
    pass


class RestartTopContext(Exception):
    pass


class FullRestartException(Exception):
    def __init__(self, reason="end_game"):
        Exception.__init__(self, reason)
        self.reason = reason


class UtterRestartException(Exception):
    pass


class QuitException(Exception):
    def __init__(self, relaunch=False, status=0):
        Exception.__init__(self)
        self.relaunch = relaunch
        self.status = status


class JumpException(Exception):
    pass


class JumpOutException(Exception):
    pass


class CallException(Exception):
    from_current = False

    def __init__(self, label, args, kwargs, from_current=False):
        Exception.__init__(self)
        self.label = label
        self.args = args
        self.kwargs = kwargs
        self.from_current = from_current

    def __reduce__(self):
        return (CallException, (self.label, self.args, self.kwargs, self.from_current))


class EndReplay(Exception):
    pass


class ParseErrorException(Exception):
    pass


CONTROL_EXCEPTIONS = (RestartContext, RestartTopContext, FullRestartException,
                      UtterRestartException, QuitException, JumpException,
                      JumpOutException, CallException, EndReplay,
                      ParseErrorException, KeyboardInterrupt)

context = _renpy.context
invoke_in_new_context = _renpy.invoke_in_new_context
call_in_new_context = _renpy.call_in_new_context
call_replay = _renpy.call_replay
