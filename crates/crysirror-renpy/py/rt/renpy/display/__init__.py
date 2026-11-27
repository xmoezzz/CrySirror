# SPDX-License-Identifier: MPL-2.0
"""Package of native Ren'Py runtime modules."""
import renpy.log

log = renpy.log.open("log", developer=False, append=False)
ic_log = renpy.log.open("image_cache", developer=True, append=False)
to_log = renpy.log.open("text_overflow", developer=True, append=True)

# Set when the interface starts (`renpy.display.core`).
interface = None
draw = None
can_fullscreen = False
less_imagedissolve = False
touch = False
info = None


def get_info():
    global info
    if info is None:
        import pygame_sdl2 as pygame
        pygame.display.init()
        info = pygame.display.Info()
    return info
