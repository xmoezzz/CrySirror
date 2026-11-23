# KiriKiri first-run confirmation and Rect on Windows

2026-11-23. The HF game with a fresh CrySirror profile previously stopped in
`System.confirm` during append setup. The game now presents the complete prompt
through the shared dialog service and resumes the calling script with the
selected integer. The visual check used the full prompt and an isolated test
profile. Escape chose No and the opening sequence continued without a TJS
error. The desktop launcher wrote the game path to its log as UTF-8.

The installed PC edition of AMBITIOUS MISSION and both After Episode titles
were replayed through the first-story click using their original XP3 archives.
All three produced a scene capture after reaching the script position that
previously raised `Member "Rect" does not exist`. The Release player used the
native Rect implementation. The playback used Windows Vulkan offscreen
rendering with audio disabled and isolated test profiles.

The offline locked Release desktop build succeeded. Fifteen tests passed across
`win32_dialog`, `tjs_semantics` and `font_hook`. These include nested startup
confirmation, both dialog results, Rect copy and geometry operations, and the
existing message and edit dialogs. The opt-in real-game font diagnostic was
outside this run.
