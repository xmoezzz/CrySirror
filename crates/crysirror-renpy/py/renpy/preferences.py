# SPDX-License-Identifier: MPL-2.0
"""In-session preference values for the 6.99 profile."""


class Preferences:
    def __init__(self):
        self.fullscreen = False
        self.skip_unseen = False
        self.text_cps = 0
        self.afm_time = 0
        self.afm_enable = True
        self.using_afm_enable = False
        self.voice_sustain = False
        self.mouse_move = False
        self.show_empty_window = True
        self.wait_voice = True
        self.afm_after_click = False
        self.transitions = 2
        self.video_image_fallback = False
        self.skip_after_choices = False
        self.volumes = {}
        self.mute = {}
        self.joymap = {}
        self.physical_size = None
        self.virtual_size = None
        self.renderer = "auto"
        self.performance_test = True
        self.language = None
        self.self_voicing = False
        self.emphasize_audio = False
        self.pad_enabled = True
        self.mobile_rollback_side = "disable"
        self.desktop_rollback_side = "disable"

    def set_volume(self, mixer, value):
        if value != 0:
            self.mute[mixer] = False
        self.volumes[mixer] = value

    def get_volume(self, mixer):
        if self.mute.get(mixer, False):
            return 0.0
        return self.volumes.get(mixer, 0.0)

    def set_mute(self, mixer, muted):
        self.mute[mixer] = muted
        if not muted and self.volumes.get(mixer, 1.0) == 0.0:
            self.volumes[mixer] = 1.0

    def get_all_mixers(self):
        from renpy.music import get_all_mixers
        return get_all_mixers()

    def init_mixers(self):
        for mixer in self.get_all_mixers():
            self.volumes.setdefault(mixer, 1.0)
            self.mute.setdefault(mixer, False)

    def get_mute(self, mixer):
        if mixer not in self.volumes:
            return False
        return self.mute[mixer]


class Persistent:
    def __init__(self):
        self._preferences = Preferences()
        self._seen_ever = {}
        self._seen_images = {}
        self._chosen = {}
        self._seen_audio = {}
        self._seen_translates = set()
        self._changed = dict.fromkeys(
            ("_preferences", "_seen_ever", "_chosen", "_seen_audio", "_seen_translates"), 0
        )

    def __getstate__(self):
        return dict(self.__dict__)

    def __setstate__(self, values):
        self.__init__()
        self.__dict__.update(values)


    def __getattr__(self, name):
        if name.startswith("__") and name.endswith("__"):
            raise AttributeError(name)
        return None


_merge_callbacks = {}


def register_persistent(field, callback):
    _merge_callbacks[field] = callback


def merge_field(field, older, newer, current):
    callback = _merge_callbacks.get(field)
    return newer if callback is None else callback(older, newer, current)
