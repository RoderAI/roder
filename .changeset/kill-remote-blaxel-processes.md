---
roder-api: minor
roder-tools: patch
roder-ext-runner-blaxel: minor
roder-ext-runner-docker: patch
roder-ext-runner-hosted-common: patch
roder-ext-runner-sprites: patch
roder-ext-runner-unix-local: patch
roder-ext-task-process: patch
roder-ext-webwright: patch
---

# Bound and cancel detached remote commands

Remote command requests can now carry a wall-clock process lease. Remote shell
and exec tools request provider cancellation when they time out or are dropped
by turn interruption instead of allowing detached work to continue.

The Blaxel runner starts every command as a uniquely named process with a
finite server-side keep-alive timeout, polls the process API for commands that
run beyond the synchronous 60-second window, advertises cancellation, and
force-kills the process group when Roder cancels the command.
