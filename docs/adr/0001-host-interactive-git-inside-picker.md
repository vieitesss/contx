# Host interactive Git inside the picker

Picker-initiated clone and deletion stay inside the TUI. A progressive action dialog overlays the preserved picker; Contx fields and recognized Git/SSH prompts use native controls; unknown interaction uses an embedded PTY/VT terminal. We accepted a PTY emulator and prompt state machine so visual and context continuity is preserved without reducing Git/SSH compatibility. CLI `clone` and `delete` are outside this boundary and keep inherited terminal behavior.

## Considered options

Restoring the real terminal for Git would break picker continuity. Noninteractive-only Git would drop SSH/auth compatibility. Semantic-only parsing of Git output would fail on unknown prompts. Embedding a PTY with native controls plus terminal fallback keeps both continuity and provider-independent interaction.
