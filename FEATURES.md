# Tide Terminal — salient features

- **Native Rust terminal emulator for Linux**, backed by a real PTY and the Alacritty terminal core.
- **Configurable GPU rendering**, defaulting to the iGPU, with explicit integrated/discrete/automatic/software choices in Settings or `gui --gpu`. Intel ADL GT2/Vulkan is the validated hardware; software CI checks are labelled separately.
- **Bash, Zsh and Fish support**, including existing shell configuration and native Fish suggestions or bundled Bash/Zsh suggestion integrations.
- **Input-needed alerts** for common password/passphrase, fingerprint-reader and confirmation prompts. Detection is heuristic; Tide does not authenticate or automatically answer prompts.
- **Command stop conditions**: elapsed runtime, output inactivity, literal output text, and regex patterns. Graceful interruption can escalate to a forced stop after a configurable grace period.
- **`T_:` timer command**: for example `T_: 'sleep 60' : -s 10`, with seconds/minutes, output rules and direct-argv support.
- **Retrospective command appending** through a dialog or `A_:` after completion. Filter retained output with `|`, append it to a file with `>>`, or run a follow-up after success with `&&`. The source command is not rerun. The dialog can also queue an action while a source runs.
- **Separate stdout/stderr capture for new sessions**, with `A_: '1>> stdout.txt'`, `A_: '2>> errors.txt'`, and stream-specific filters. No source wrapper is required; original redirections and direct controlling-terminal writes still determine the channel.
- **Tabs and command history**, including search, lookup and insertion into the shell editor.
- **Standard terminal input and clipboard controls**, including Ctrl+C, suspend/resume and common legacy control characters.
- **Visual controls** for opacity, font sizing and cursor animation, plus bundled Nerd Font icons, Braille, box-drawing and Unicode symbol fallbacks.
- **Private sessions** that disable Tide history recording. Explicit file output still writes the file the user requests.
- **Automated regression library** covering terminal parsing, chunk boundaries, PTY replies, shell integration, prompts, process lifecycle, timers, output capture, appending and real iGPU GUI interaction.

Capture is bounded normalized terminal text, not a byte-exact archival stream. Full-screen applications, explicitly redirected streams and direct controlling-terminal I/O require attention to the capture contract. Current validation reports also identify known upstream terminal-parser deviations instead of claiming full VT conformance.
