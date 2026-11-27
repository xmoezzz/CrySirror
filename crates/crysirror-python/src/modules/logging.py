# SPDX-License-Identifier: MPL-2.0
"""Named loggers and stream handlers for the embedded Python VM."""
import sys

NOTSET = 0
DEBUG = 10
INFO = 20
WARNING = WARN = 30
ERROR = 40
CRITICAL = FATAL = 50

_names = {DEBUG: "DEBUG", INFO: "INFO", WARNING: "WARNING", ERROR: "ERROR", CRITICAL: "CRITICAL"}


class StreamHandler:
    def __init__(self, stream=None):
        self.stream = sys.stderr if stream is None else stream
        self.level = NOTSET

    def setLevel(self, level):
        self.level = level

    def emit(self, level, name, message):
        self.stream.write(message + "\n")
        self.stream.flush()


class Logger:
    def __init__(self, name):
        self.name = name
        self.level = NOTSET
        self.handlers = []
        self.propagate = True

    def setLevel(self, level):
        self.level = level

    def addHandler(self, handler):
        if handler not in self.handlers:
            self.handlers.append(handler)

    def removeHandler(self, handler):
        if handler in self.handlers:
            self.handlers.remove(handler)

    def _parent(self):
        if self is root:
            return None
        name = self.name
        while "." in name:
            name = name.rsplit(".", 1)[0]
            if name in _loggers:
                return _loggers[name]
        return root

    def getEffectiveLevel(self):
        logger = self
        while logger is not None:
            if logger.level != NOTSET:
                return logger.level
            logger = logger._parent()
        return NOTSET

    def isEnabledFor(self, level):
        return level >= self.getEffectiveLevel()

    def log(self, level, message, *args):
        if not self.isEnabledFor(level):
            return
        message = str(message)
        if args:
            message = message % args
        logger = self
        handled = False
        while logger is not None:
            for handler in logger.handlers:
                handled = True
                if level >= handler.level:
                    handler.emit(level, self.name, message)
            if not logger.propagate:
                break
            logger = logger._parent()
        if not handled and level >= WARNING:
            sys.stderr.write(message + "\n")

    def debug(self, message, *args):
        self.log(DEBUG, message, *args)

    def info(self, message, *args):
        self.log(INFO, message, *args)

    def warning(self, message, *args):
        self.log(WARNING, message, *args)

    warn = warning

    def error(self, message, *args):
        self.log(ERROR, message, *args)

    def critical(self, message, *args):
        self.log(CRITICAL, message, *args)

    fatal = critical


root = Logger("root")
root.setLevel(WARNING)
_loggers = {}


def getLogger(name=None):
    if not name or name == root.name:
        return root
    if name not in _loggers:
        _loggers[name] = Logger(name)
    return _loggers[name]
