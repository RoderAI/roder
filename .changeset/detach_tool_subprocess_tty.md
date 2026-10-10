---
roder-tools: patch
---

# Run shell and exec tool subprocesses in their own session

Tool subprocesses inherited roder's controlling terminal. An interactive
shell started by a tool (for example `zsh -ic '...'`) took over the terminal's
foreground process group, so roder's next read from the keyboard stopped the
whole TUI with SIGTTIN and the session appeared suspended and then died.
`shell`, `exec_command`, and `write_stdin` sessions now call `setsid`, so those
commands have no controlling terminal and cannot steal the TUI's.
