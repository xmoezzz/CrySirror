# SPDX-License-Identifier: MPL-2.0
"""Transition definitions used by the native with statement."""


class Dissolve:
    def __init__(self, time, alpha=False, time_warp=None):
        self.time = float(time)
        self.alpha = alpha
        self.time_warp = time_warp
