# Ren'Py 6.99.12.4 native contract

The installed-game plan reads `renpy/__init__.py` for the four numeric
release fields. Version 6.99.12.4 selects the Python 2 dialect. RPC2 slot
decoding is separate from the runtime version. The script test corpus also
exercises the modern source grammar without selecting a modern runtime.

Primary evidence is the 6.99.12.4.2187 tag at
`ee751a2c3ba6f2b2cdd5209233225db41b844520` under
`local/reference/renpy` and the original DDLC 6.99.12.4 installation. The
player mounts RPA assets in reverse archive name order and indexes loose
files before mounted members. The 6.99 loader prefers a compiled script
whose trailing MD5 matches the normalized source bytes when both exist.
`_errorhandling` and the optional `tl/None/common` boot modules run before
ordinary scripts.

| Native family | State and host boundary |
| --- | --- |
| `Label`, `Call`, `Jump`, `Return` | Arena node destinations and return sites. Binding a called label changes the script store and saves earlier bindings in the dynamic scope. Return restores that scope. |
| `Python`, early Python, `Define`, `Default` | Workspace Python VM and per-engine module namespaces. Named stores retain their own values. Early code registers runtime declarations and defaults are applied at the native startup call. |
| `Say`, `Menu`, `renpy.pause` | A suspended Python task keeps its continuation. Filtered menu rows retain their source branch indexes. Pause resumes with a boolean that distinguishes dismissal from timeout. CrySirror owns input and the display clock. |
| `renpy.call_in_new_context`, `renpy.dynamic` | A nested label receives its own context and disabled rollback, and returns a value to the suspended Python task. Dynamic bindings restore on return. |
| `UserStatement` | Registered lexer and callback invocation. The source parser matches the longest registered prefix. The optional next callback selects the next label. |
| `Image`, `Show`, `Scene`, `Hide` | Native names and tags map to shared retained image, solid and text layers. Parameterized text evaluates its source expression in the script store. |
| ATL and `Dissolve` | Sequential ATL placement, scaling, opacity and child steps use logical time and retained layer state. Cross-dissolve uses the shared scene effect. Additional ATL control forms and transition families have separate semantics. |
| `Screen`, `Style` | SL2 expression scope, parameters, conditional nodes, loops and screen inclusion feed shared retained widgets. The host supplies activation events. The script VM invokes the original authored actions and resumes screen waits with their result. |
| `renpy.music`, `renpy.sound` | Channel IDs and aliases are adapter-owned. The host prepares and plays a scoped audio asset and owns the sample clock, loop and fades. Synchronized/tight starts, queues, custom clip ranges and video remain unsupported. |
| Python temporary files and leases | `/tmp/<file>` maps to a client-cache entry. Exclusive nonblocking `lockf` uses a client storage lease held by the Python file object. Closing or unlocking releases the lease. |
| Rollback | Suspended dialogue and menu checkpoints retain the store, dynamic state, tasks and scene. Mutable objects restore in place. Roll-forward resumes from the recorded decisions. Persistent state and objects reachable solely through `NoRollback` remain outside the checkpoint. |
| `save_persistent` | Scoped `persistent` and recovery `persistent.new` use the native zlib-wrapped pickle container. The decoder restricts class resolution and atomically writes the client save file. Original ZIP game slots require serializing the native rollback log, contexts and roots. |

Native calls are Python module members, constructors and dynamically registered
statements. This profile has no independent numbered syscall table. Registered
or parsed classes alone give no execution coverage count. See the
[macOS validation record](../validation/renpy.md) for exercised paths.
