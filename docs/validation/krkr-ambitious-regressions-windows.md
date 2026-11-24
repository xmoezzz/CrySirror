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
