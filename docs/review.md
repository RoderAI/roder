# Roder Code Review

`/review` runs a read-only reviewer over a change and returns structured
findings. The findings are rendered in the TUI, echoed into the requesting
thread's transcript, and can be submitted to an external platform through a
pluggable review publisher — the first-party one posts them as GitHub pull
request review comments.

Nothing publishes automatically. A review produces findings; submitting them is
always an explicit action.

## Slash Command

```
/review                       # the uncommitted working diff (default)
/review --uncommitted         # the same, spelled out
/review --base main           # everything this branch adds on top of main
/review --commit 9f2c1ab      # one commit
/review focus on the retry loop in the runner
/review --base main --publish github
```

- Exactly one target may be given. `--base`, `--commit`, `--uncommitted`, and
  free-form instructions are mutually exclusive; combining them is an error
  rather than a silently dropped argument.
- An unrecognized `--flag` is an error, not review instructions.
- `--publish <id>` does **not** publish. It preselects which publisher the
  review panel uses when you press `p`.

`/review` is a local RPC command in the TUI: it calls `review/start` with
detached delivery and returns immediately, so the composer stays responsive
while the reviewer works. Findings arrive later as a `review/completed`
notification.

A review cannot start while the requesting thread has an active turn. The
runtime rejects it with `thread <id> has an active turn (<turn id>); wait for
it to finish before starting a review`.

## Targets

| Target | JSON | Prompt |
| --- | --- | --- |
| Uncommitted changes | `{ "kind": "uncommittedChanges" }` | Review staged, unstaged, and untracked changes. |
| Base branch | `{ "kind": "baseBranch", "branch": "main" }` | Review what this branch would merge into `main`. |
| Commit | `{ "kind": "commit", "sha": "9f2c1ab", "title": "..." }` | Review one commit. |
| Custom | `{ "kind": "custom", "instructions": "..." }` | Free-form scope. |

Targets are resolved against the workspace's version control provider (the
`VcsProvider` trait, not `git` directly). For a base branch, the provider's
known merge base is used when its ref actually refers to the requested branch
(`origin/main` and `main` are treated as the same branch); otherwise the prompt
asks the reviewer to derive the merge base itself with `git merge-base`.

Version control failures are non-fatal. The prompt degrades to the
derive-it-yourself form, and `baseSha`/`headSha` are left unset so a publisher
reports an unanchored review instead of anchoring to the wrong revision.

Every prompt ends with the repository root, because the output schema requires
absolute file paths.

## How the Review Turn Runs

Roder has no one-shot sub-agent with a per-call system prompt, so a review is:

1. A **detached child thread** of the requesting thread (`rootId` = the
   requesting thread) titled `review: <label>`.
2. A **single turn** on that thread whose `InstructionBundle.system` is replaced
   by the review rubric (`crates/roder-core/src/review/rubric.md`).
3. Collected by watching the event bus for that turn's `final_answer`-phase
   assistant message, terminating on turn completion or failure.

The review thread is created with a **read-only tool allowlist**:

```
read_file, list_files, grep, glob, shell
```

`shell` is present because `git diff` / `git log` is how a reviewer sees a
change. Every tool that writes files, edits code, applies patches, runs
processes, or spawns agents is absent. The allowlist is enforced both when tools
are advertised to the model and when a call is dispatched. The global policy
mode is never touched, so a review can never widen what the parent thread may
do.

When the review finishes, two items are appended to the **requesting** thread's
transcript: a synthetic `<user_action>` user message carrying the findings, and
an assistant message with the rendered summary. This is what lets the parent
agent act on findings you keep. Echoing is best effort — a persistence failure
does not fail the review.

Completed reviews are kept in memory (most recent 64) so `review/publish` can
act on a review id alone. **Review history does not survive a restart**;
publishing an id from an earlier process fails with a clear error.

## The JSON Contract

Roder has no structured-output plumbing, so the rubric instructs the model to
emit a single JSON object as its final message and the runtime parses it with a
fallback ladder:

1. Parse the whole message as JSON.
2. Strip ```` ```json ```` fences and parse.
3. Extract the first balanced `{…}` substring and parse.
4. Give up and return `{ overallExplanation: <raw text>, findings: [] }`.

Before deserializing, tiers 1–3 normalize the decoded value: snake_case keys
(`code_location`, `absolute_file_path`, `line_range`, …) are renamed to the
camelCase schema, and `priority` is coerced from a numeric `0..3` or an
uppercase `"P1"`. An unrecognized priority is dropped so the field falls back to
its default.

A unit test extracts the rubric's fenced JSON example and round-trips it through
the parser, so drift between the prompt and the type fails the build instead of
silently emptying every review.

### Finding schema

```json
{
  "findings": [
    {
      "title": "[P1] Retry loop never resets the backoff",
      "body": "Markdown explaining why this is a problem.",
      "confidenceScore": 0.82,
      "priority": "p1",
      "codeLocation": {
        "absoluteFilePath": "/Users/me/project/crates/runner/src/retry.rs",
        "lineRange": { "start": 120, "end": 124 }
      }
    }
  ],
  "overallCorrectness": "patch is correct",
  "overallExplanation": "1-3 sentences justifying the verdict.",
  "overallConfidenceScore": 0.82
}
```

- `findings` is required; `[]` means nothing worth flagging.
- `codeLocation` is required on every finding. `absoluteFilePath` must be
  absolute; `lineRange` is inclusive and 1-based in the post-change file.
- `priority` is `"p0"` | `"p1"` | `"p2"` | `"p3"` and defaults to `"p2"`.
  `p0` is "drop everything", `p3` is "nice to have".
- `confidenceScore` and `overallConfidenceScore` are floats in `0.0..=1.0`.
- `overallCorrectness` is `"patch is correct"` or `"patch is incorrect"`.

## TUI Review Panel

When `review/completed` arrives for the current thread, the findings render into
the timeline **and** the review panel opens. The panel is dismissable; the
transcript entry is not. Every finding starts **kept** — you drop noise rather
than rebuilding the reviewer's list by hand.

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Move the cursor |
| `PageUp` / `PageDown` | Move ten findings |
| `Home` / `End` | First / last finding |
| `Space` | Toggle keep on the finding under the cursor |
| `a` | Keep all |
| `d` | Drop all |
| `Enter` | Toggle the detail view for the selected finding |
| `p` | Publish the kept findings |
| `Esc` | Leave the detail view, or close the panel |
| `Ctrl+C` | Close the panel and open the exit dialog |

`p` issues `review/publish` with the kept indexes in report order and the
publisher id from `--publish <id>` when one was given. The result — or the
error — is shown in the panel's status line.

Priority colors reuse the existing theme helpers: P0 error, P1 shell, P2 accent,
P3 muted. Theming tokens for the surface are `#review-panel`,
`.review-finding`, `.review-finding-kept`, and
`.review-finding[data-priority="p0".."p3"]`.

## Publishers

A publisher is an ordinary extension service. Core knows nothing about GitHub:
`Runtime::publish_review` resolves a publisher by id from the extension registry
and calls it through the `ReviewPublisher` trait.

```rust
#[async_trait]
pub trait ReviewPublisher: Send + Sync + 'static {
    fn descriptor(&self) -> ReviewPublisherDescriptor;
    async fn is_available(&self, workspace_root: &Path) -> bool { true }
    async fn publish(&self, request: ReviewPublishRequest)
        -> Result<ReviewPublishResult, ReviewPublishError>;
}
```

Register one from `RoderExtension::install`:

```rust
registry.review_publisher(Arc::new(MyPublisher::new(config)));
```

and declare `ProvidedService::ReviewPublisher("my-publisher".into())` in the
extension manifest — the host validates that a manifest declares what it
actually installs. Shipping a third-party publisher needs no core changes.

### Adding a platform

`roder-config` stores every `[review.publishers.<id>]` block as an opaque
`toml::Value` keyed by publisher id, and exposes it as
`ReviewConfig::publisher(id)`. The shape of a block belongs to the publisher's
own crate, so adding a platform never edits a core config struct. A publisher
crate parses its own block:

```rust
// crates/roder-ext-vex-review/src/config.rs
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VexReviewConfigToml { pub endpoint: Option<String>, /* … */ }

impl VexReviewConfig {
    pub fn from_toml(value: &toml::Value) -> Result<Self, String> { /* … */ }
}
```

Wiring a new platform is then three mechanical steps:

1. Add a `crates/roder-ext-<platform>-review` crate implementing
   `ReviewPublisher` and parsing its own `[review.publishers.<id>]` block.
2. Add one arm in `crates/roder-extension-host/src/review.rs` to hand that block
   to the crate.
3. Install it in `build_default_registry`.

Nothing in `roder-api`, `roder-core`, `roder-protocol`, `roder-app-server`, or
the TUI changes. `review/publishers/list` picks the new publisher up
automatically, and `--publish <id>` can select it. A block for a publisher that
is not installed is ignored rather than rejected, so config stays forward
compatible.

The GitHub publisher declares `process.exec.gh`, `network.api.github.com`, and
`secret.read.<token_env>` as required capabilities.

`ReviewDestination` is deliberately free-form — `{ publisherId, target, options }`
— so each publisher interprets `target` (a PR reference, an issue id, a channel)
and `options` however it needs.

Publisher selection precedence:

1. `publisherId` on the `review/publish` request (or `--publish <id>` in the
   TUI, which fills it in).
2. `[review].default_publisher`.
3. The only installed publisher, when exactly one is installed.
4. Otherwise an error naming the installed candidates.

`is_available` is advertised but not used to filter selection: an unconfigured
publisher returns its own precise "not configured" error rather than silently
disappearing from the list.

## GitHub Publisher

`roder-ext-github-review` is installed by default and registers under the id
`github`. It submits `POST /repos/{owner}/{repo}/pulls/{number}/reviews`.

### Setup

Either works; `mode = "auto"` prefers the CLI because it needs no configuration.

- **CLI** — install the [GitHub CLI](https://cli.github.com) and run `gh auth
  login`. Roder shells out to `gh pr view`, `gh api --paginate`, and
  `gh api --method POST … --input -`.
- **HTTP** — export a token in `GITHUB_TOKEN` (or whatever `token_env` names).
  Requests carry `Authorization: Bearer`, `Accept: application/vnd.github+json`,
  and `X-GitHub-Api-Version: 2022-11-28`.

If neither is usable, publishing fails with a configuration error naming both
ways out.

### Destination

Omit `destination.target` and the publisher resolves the pull request for the
current branch from the workspace. To be explicit, pass any of:

```
owner/repo#123
owner/repo/123
https://github.com/owner/repo/pull/123
```

### Mapping findings to comments

The submitted body is
`{ commit_id, event, body, comments: [{ path, side, line, start_line?, start_side?, body }] }`
— GitHub's own snake_case field names, not the camelCase of `ReviewFinding`.

- `path` is `absoluteFilePath` relativized to `git rev-parse --show-toplevel`
  (forward slashes, no leading `./`). The git root, not the workspace root —
  worktree forks make these differ.
- `line` is `lineRange.end`; `start_line`/`start_side` are emitted only when
  `start < end` **and** both ends land in the same diff hunk. Otherwise the
  comment is single-line rather than dropped.
- `side` and `start_side` are always `RIGHT` (the post-change file).
- `commit_id` is the PR's `headRefOid`. GitHub rejects a review anchored to any
  other commit.
- A comment is only emitted when its line falls inside a diff hunk's **full
  new-file span, including context lines** — `new_start ..= new_start + new_count - 1`
  parsed from each `@@` header of the file's `patch`. Files with no `patch`
  (binary, or too large) have no commentable lines.
- This prefilter is mandatory, not best effort: GitHub 422s the **entire**
  review if any single comment is unmappable. Unanchorable findings are moved
  into the summary body as bullets and reported in `skipped`.
- Findings below `min_priority` are recorded in `skipped` and **not** echoed
  into the body — they were deliberately suppressed.
- Comment bodies are truncated at 60k characters.
- On rejection, GitHub's `errors[]` are surfaced verbatim; those messages
  ("Line could not be resolved") are the only actionable signal.

Set `dryRun: true` to get the exact request body back in `payloadPreview` with
no network write. Always worth doing the first time against a real PR.

## Configuration

```toml
[review]
default_publisher = "github"   # omit, or "none", to require an explicit choice
model             = "opus"     # optional model override for review turns only

[review.publishers.github]
mode           = "auto"        # auto | cli | http
event          = "COMMENT"     # COMMENT | REQUEST_CHANGES | APPROVE
min_priority   = "p2"          # least severe priority still published
token_env      = "GITHUB_TOKEN"
gh_bin         = "gh"
api_base_url   = "https://api.github.com"
timeout_seconds = 20
```

- `default_publisher` accepts `"none"` or an empty string to mean "always ask".
- `model` applies only to the review turn; the requesting thread keeps its own
  model.
- `event` defaults to `COMMENT`, which never changes a pull request's state.
  Approving or requesting changes on your behalf is opt-in. A single publish can
  override it with `destination.options.event`.
- `min_priority` defaults to `p3`, i.e. publish everything. `p2` drops `p3`
  findings.
- `api_base_url` covers GitHub Enterprise.
- Unknown values for `mode`, `event`, or `min_priority` fail the registry build
  rather than being silently ignored — a typo in `REQEUST_CHANGES` must not
  quietly downgrade a review to a plain comment.

## API

See `docs/app-server/api.md` for the `review/start`, `review/publish`, and
`review/publishers/list` methods and the `review/started`, `review/completed`,
`review/failed`, and `review/published` notifications.
