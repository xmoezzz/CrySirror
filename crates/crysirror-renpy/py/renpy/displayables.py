# SPDX-License-Identifier: MPL-2.0
"""Script-facing displayable values. Presentation is owned by CrySirror."""
import _renpy


class Displayable:
    def __init__(self, style="default", **properties):
        self.style_name = style
        self.properties = properties


class Null(Displayable):
    def __init__(self, width=0, height=0, **properties):
        super(Null, self).__init__(**properties)
        self.width = width
        self.height = height


class Solid(Displayable):
    def __init__(self, color, **properties):
        super(Solid, self).__init__(**properties)
        self.color = Color(color)


class Image(Displayable):
    def __init__(self, filename, **properties):
        super(Image, self).__init__(**properties)
        self.filename = filename


class ImageReference(Displayable):
    def __init__(self, name, **properties):
        super(ImageReference, self).__init__(**properties)
        self.name = tuple(name.split()) if isinstance(name, str) else tuple(name)


class Text(Displayable):
    def __init__(self, text, **properties):
        super(Text, self).__init__(**properties)
        self.text = text


class ParameterizedText(Displayable):
    def parameterize(self, parameters):
        import store
        if len(parameters) != 1:
            raise ValueError("ParameterizedText takes one expression")
        text = eval(parameters[0], store.__dict__)
        return Text(text, style=self.style_name, **self.properties)


class ImageModule:
    Image = Image


im = ImageModule()


def Color(value):
    if isinstance(value, str) and value.startswith("#"):
        hex = value[1:]
        if len(hex) in (3, 4):
            hex = "".join(c + c for c in hex)
        if len(hex) == 6:
            hex += "ff"
        if len(hex) == 8:
            return tuple(int(hex[i:i + 2], 16) for i in range(0, 8, 2))
    elif isinstance(value, (tuple, list)) and len(value) in (3, 4):
        return tuple(value) if len(value) == 4 else tuple(value) + (255,)
    raise ValueError("invalid Ren'Py color")


class ScreenStatement:
    def __init__(self, name, displayable, children, style, scope=False, replaces=False,
                 default_keywords=None, custom=False):
        self.handle = _renpy.register_sl(name, displayable, children, style, scope, replaces,
                                         default_keywords, custom)
        self.properties = set()
        self.groups = []

    def add_positional(self, name):
        _renpy.add_sl_positional(self.handle, name)
        return self

    def add_property(self, name):
        self.properties.add(name)
        return self

    def add_style_property(self, name):
        return self.add_prefix_style_property("", name)

    def add_prefix_style_property(self, prefix, name):
        for state in ("", "insensitive_", "idle_", "hover_", "selected_",
                      "selected_insensitive_", "selected_idle_", "selected_hover_"):
            self.properties.add(prefix + state + name)
        return self

    def add_property_group(self, group, prefix=""):
        if group not in ("bar", "box", "button", "position", "text", "ui", "window"):
            raise ValueError("unknown screen property group " + group)
        self.groups.append((group, prefix))
        return self


def register_sl_displayable(name, displayable, nchildren=0, style=None, scope=False,
                            replaces=False, default_keywords=None):
    return ScreenStatement(name, displayable, nchildren, style, scope, replaces, default_keywords)


def register_sl_statement(name, positional, nchildren):
    statement = ScreenStatement(name, None, nchildren, None, custom=True)
    for i in range(positional):
        statement.add_positional(str(i))
    return statement
