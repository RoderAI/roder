# Roder Project Knowledge Base

Roder keeps a per-project knowledge base of durable documents — requirements,
decisions, research notes, runbooks, long-form memory narratives, artifact
references, and free-form notes — that the agent can list, search, read, and
write during runs, and that humans can browse and edit as plain markdown.

The first engine, `roder-ext-knowledge-md`, is markdown-file based: every
document is a markdown file with YAML front matter, so the knowledge base is
human-readable, diffable, and editable outside Roder. A future engine will
add embedding-backed semantic recall and automatic knowledge reconciliation
(duplicate merging, contradiction surfacing, staleness review) behind the same
`KnowledgeStore` contract; see `roadmap/93-roder-project-knowledge-base.md`.

## Storage layout

```text
~/.roder/knowledge/
  project-<key>/
    docs/<kind>/<slug>.md        # document heads
    revisions/<doc-id>/<rev>.md  # immutable prior revisions
  global/
    docs/<kind>/<slug>.md
```

- The project key defaults to the workspace directory name (the same
  resolution `roder memory` uses).
- Documents edited out-of-band are picked up on the next read; the content
  hash is always derived from the body on load.
- Updates never overwrite history: each update snapshots the prior head into
  `revisions/` and bumps the revision number.
- Deleting archives: archived documents leave lists and search but stay
  readable by id and on disk.

A document looks like:

```markdown
---
id: kn-928fdb4644d2
kind: decision
title: Use markdown knowledge engine
status: active
source: user
tags:
- adr
revision: 2
created_at: 2026-06-12T03:08:02Z
updated_at: 2026-06-12T03:08:29Z
---

Updated decision body.
```

## Document model

- Kinds: `memory`, `requirement`, `decision`, `research`, `runbook`,
  `artifact`, `note`, or any custom string.
- Status: `active`, `draft`, `superseded`, `archived`.
- Source attribution: `user` (CLI/app-server), `agent` (tools),
  `reconciler` (future), `import`.
- Typed links between documents: `relates_to`, `supersedes`, `derived_from`,
  `contradicts`, `duplicates`.
- Scopes: project (default) and global. Global recall folds into project
  search only when requested.

## Agent access during runs

The agent gets seven tools when knowledge is enabled:

```text
knowledge_list      # browse by kind/tag/status
knowledge_read      # read by id, line-paginated, optional prior revision
knowledge_search    # scored snippets with document ids
knowledge_save      # create a document (kind, title, body, tags)
knowledge_update    # revise body/title/status/tags; writes a revision
knowledge_delete    # archive
knowledge_link      # add/remove typed links
```

Prompt-time recall injects up to `[knowledge].recall_limit` (default 4)
relevant documents per turn as bounded, cited `Knowledge` context blocks that
point the model at `knowledge_read` for full bodies. Recall failures degrade
to no blocks and never abort a turn.

## CLI

```sh
roder knowledge list [--scope project|global|project:<id>] [--kind decision] [--tag api] [--status active]
roder knowledge read kn-928fdb4644d2 [--revision 1]
roder knowledge search "session tokens" [--include-global]
roder knowledge save --kind decision --title "Use markdown" --tag adr "Body text"
echo "Body from stdin" | roder knowledge save --kind research --title "Benchmarks"
roder knowledge update kn-928fdb4644d2 --status superseded "New body"
roder knowledge link kn-a kn-b --type supersedes [--remove]
roder knowledge revisions kn-928fdb4644d2
roder knowledge delete kn-928fdb4644d2
```

## TUI

- `/knowledge` lists project documents; `/knowledge list <kind>`,
  `/knowledge search <text>`, and `/knowledge read <id>` browse the corpus.
- The command palette (`Ctrl+K`) has Knowledge list/search/read entries.

## App-server API

`knowledge/list`, `knowledge/read`, `knowledge/save`, `knowledge/update`,
`knowledge/delete`, `knowledge/search`, `knowledge/links/set`, and
`knowledge/revisions/list`, with `knowledge/saved`, `knowledge/updated`,
`knowledge/archived`, and `knowledge/linked` notifications. See
`docs/app-server/api.md` for request/response shapes.

## Configuration

```toml
[knowledge]
enabled = true        # install the knowledge extension
backend = "markdown"  # only engine today
store_path = ""       # default: ~/.roder/knowledge
recall = true         # inject relevant knowledge into turns
recall_limit = 4
```

## Relationship to memories

Memories (`memory_*`, `memory/*`) stay what they are: short atomic facts with
embedding-backed recall. Knowledge documents are larger, titled, kind-tagged
markdown artifacts with revisions and links, listable and navigable as a
corpus. There are no compatibility aliases between the two surfaces.

## Privacy

The markdown engine is fully local: document content never leaves the machine
through this feature. Recall injects document snippets into model prompts, so
knowledge content reaches your configured inference provider the same way any
prompt context does.
