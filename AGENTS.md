# Project workflow

The user requests a commit and GitHub push after each completed, tested project change. Use the configured origin and current project branch. Do not force push or install an unattended file-watching push service.

Run task-relevant tests while developing. Before publishing a completed application change, run formatting, strict Clippy, all Rust targets with one test thread, and the CLI/timer smoke suites. The aggregate command is documented in tests/README.md. Run the real iGPU GUI suite locally for terminal, shell transport, rendering or input changes when the isolated test hardware is available. Hosted CI deliberately does not claim those hardware checks.

Keep the integrated-GPU requirement. Preserve Bash, Zsh and Fish compatibility, private-session behavior, command exit status and process-group control. Retrospective appending must not rerun the source command. Include applicable bundled-font/vendor licenses. Keep generated captures, user output files, databases, credentials and build products out of Git.
