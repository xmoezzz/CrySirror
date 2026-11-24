# AMBITIOUS MISSION image and save regression follow-up

2026-11-24. Built the Windows player in locked offline Release mode with
LTO disabled and 16 codegen units. Verified that the player SHA-256 differs
from the earlier binary that reproduced both failures. Both replays used
the same input tracks as the prior AM validation build, separate copies
of the test saves, Vulkan offscreen rendering and no audio device.

| Edition and action | Implementation | Behavior on the fresh Release player |
| --- | --- | --- |
| Main story, long Ctrl hold | Reserve the destination plus the peak decoded source during ordered composition. Add graph-owned encoded buffers separately and retain the 256 MiB composition limit. | Completed 15,600 frames, past the prior image composition limit failure, including the scripted Ctrl release. |
| After Episode 1, open Save during a transition and confirm slot 1 | Freeze the nearest crossfade endpoint in the destination root's local coordinates. Leave live transition state and the save format intact. | Completed 15,000 frames and reached the save page wait. The updated slot contains a 276 × 155 JPEG that decodes successfully. |

The snapshot test covers both transition halves, unchanged elapsed progress
and rejection of a crossfade that crosses an unrelated capture boundary.
The image budget test covers large ordered overlays without cumulative
charges for temporary decoded surfaces. The Windows test saves and captures
remain local.

## Backlog scene jump

The report from the main story stopped during Jump preview in
`Layer.clipAlphaRect`, which needs a resumable paint call. A Windows replay
of the same action on the prior player reproduced a separate rejection of an
`ltPsScreen` layer in `Layer.piledCopy`. The composed copy now lets the
paint call finish before capturing the scene. The copied subtree and each
crossfade endpoint keep the screen blend within their own composition.

After confirmation, the replay exposed a missing mutable AlphaMovie surface.
Canvas draws now use an immutable frame from the shared media service.
Stopping or removing the movie releases its own reference. Current layers and
frozen copies retain the picture until their final use.

The rebuilt Windows Release player completed 8,551 frames in the Vulkan
offscreen replay. The input opened Backlog, previewed an earlier line,
confirmed the jump and advanced once. The trace recorded the selected line
followed by the next one, and the renderer captured the following transition.
The replay used a separate save copy and disabled audio output. The
`clipAlphaRect` callback order was verified with a synthetic layer test.

## Expression overlays in slots 0004 and 0005

The Mode A AJPM decoder now uses the separate compressed alpha plane and
clears pixels outside the current frame rectangle. A local Windows Release
replay of slot 0004 captured the question-mark animation and the blue bubbles
from `emo15_2.amv`. At ticks 7936 and 7977, the previous player displayed an
opaque black rectangle around the blue bubbles, and the rebuilt player
preserved the face and classroom between them. These captures used 10 ms of
wall pacing per offscreen tick so the movie decoder could keep up with the
simulated clock. The codec tests cover partial alpha, a moving rectangle,
a blank frame, and a mask length that disagrees with the rectangle.

During slot 0005 restore, the portrait and its frame require several paint
callbacks and resource waits. Starting `clipAlphaRect` retains the last
complete desktop snapshot through the current paint pass. A synthetic test
waits for frame art after the first mask finishes and confirms that the
presentation hold lasts until both callbacks complete.

The real save replay captured the unfinished portrait composition at tick
7000 and recorded `presentation_pending: true`. The raw offscreen capture
bypasses the desktop presentation gate. At tick 7250 the hold had cleared and
only the portrait inside the cut-in frame was visible. These checks used local
save copies and Vulkan offscreen rendering.

## Portrait colors after expression redraws

Slot 0007 restores the park dialogue. The same scene from slot 0006 showed
progressive desaturation after repeated expression changes on the earlier
Windows player. The native trace paired a 299 × 294 drawing clip with
`Layer.modulate(0, -14, 0)` on a copy of the previous portrait. The shared
HSL effect now takes that clip and updates RGB inside it.

On the rebuilt Windows Release player, the same two captures at ticks 7500
and 8220 kept the median per-pixel RGB channel spread at 59 in an unchanged
region of the blue jacket. On the previous player those captures measured
50 and 36. A long Ctrl replay from slot 0005 reached the same park dialogue.
At tick 19200 the old jacket measured 11 and the new jacket still measured 59.
The direct load of slot 0007 and both scripted Ctrl holds and releases
completed. These replays used copied saves and the Vulkan offscreen renderer
with audio output disabled.
