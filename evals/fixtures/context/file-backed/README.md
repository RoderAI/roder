# File-backed dynamic context eval fixtures

Offline fixtures for roadmap phase 53. Each case ships:

- `fixture.json` — prompt, expected answer, grading rules, metric flags
- `inline_context.txt` — bounded model-visible context with artifact references
- `artifacts/*` — full recoverable payload (long command output or chat history)

## Cases

| Id | Scenario | Answer only in |
|----|----------|----------------|
| `long-command-secret-line` | Deploy token buried in 400+ line build log | `artifacts/cmd_build.stdout.txt` (line 217) |
| `compaction-history-recovery` | Full session UUID dropped from lossy summary | `artifacts/history_turn_3.chat_history.txt` |

Run graders (when `roder-evals` is available):

```sh
cargo test -p roder-evals file_backed_context
```
