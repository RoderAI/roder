# File-Backed Dynamic Context Benchmark Results

- Fixture dir: `evals/fixtures/context/file-backed`
- Offline: `true`
- Generated: `2026-05-20T20:02:35.192456Z`

| Fixture | Correct | Inline Chars Before | Inline Chars After | Tokens Before | Tokens After | Tokens Saved | Artifact Bytes | Artifact Lines | Reads | Greps | Tails | Turn ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `compaction-history-recovery` | true | 104244 | 455 | 26061 | 114 | 25947 | 104244 | 900 | 0 | 1 | 0 | 3 |
| `long-command-output` | true | 182364 | 447 | 45591 | 112 | 45479 | 182364 | 2400 | 0 | 1 | 0 | 4 |

## Recovered Details

- `compaction-history-recovery`: `617: {"turn":617,"role":"assistant","text":"project codename: night-jar"}`
- `long-command-output`: `1937: RECOVERY_TOKEN=roder-file-backed-context`
