---
date: 2026-06-01
topic: version-control-extension
---

# Version Control Extension Requirements

## Summary

Roder should expose version control as a first-class extension provider subsystem, with git shipped as the bundled default provider rather than embedded as special core behavior. The initial scope covers the full agent workflow around review, change selection, provider history snapshot creation, restore, line switching, and sync where the active provider supports those operations.

---

## Problem Frame

Roder currently has git-specific behavior in core agent workflows and app-server surfaces. Live change review is exposed through `git/changes/list` and `git/changes/read`; workspace observations are described as git-reconciled changes; status and the built-in commit workflow also assume git language. That is workable for a git-only product, but it conflicts with Roder's extension direction: providers should own replaceable subsystems, and the harness should not require forks when a downstream distribution wants a different implementation.

Supporting jj, Subversion, or another version control system is not just a command substitution problem. Git has a staging/index model, jj has a different change and operation model, and SVN has a different working-copy and remote story. A durable abstraction needs to normalize agent outcomes instead of making every provider impersonate git.

---

## Key Decisions

- **Version control is a provider subsystem.** Roder should model VCS as a replaceable provider in the extension architecture, parallel in spirit to inference, session storage, sandbox, memory, and context providers.

- **Normalize outcomes, not git concepts.** Canonical operations should describe what the agent needs to do: inspect changes, read diffs, select work, create a durable provider history snapshot, restore work, switch lines of work, and sync with an upstream authority. Git-specific concepts like staging should appear only when the active provider exposes them as supported capabilities or provider-native extras.

- **Do not overload checkpoint terminology.** The extension API already uses `CheckpointStore` for thread/session persistence. VCS history creation should use distinct snapshot terminology, such as `vcs/snapshot/create`, so extension authors do not confuse provider commits with session checkpoints.

- **Git becomes the included provider.** The default distribution should continue to work well in git repositories, but git command execution should sit behind the provider boundary rather than being hardcoded into app-server and runtime workflow logic.

- **Provider capabilities are mandatory.** Roder clients, tools, skills, and agents need to know when an operation is unsupported, partially supported, or provider-native. Missing behavior should be visible as structured capability data, not discovered by failed shell commands.

- **Prefer a clean `vcs/*` surface.** Because this is a fast-moving new project, the canonical app-server and protocol surface should move toward version-control naming instead of preserving long-lived duplicate `git/*` APIs.

---

## Actors

- A1. **Agent runtime.** Needs reliable workspace-change awareness before and after tool calls, and needs to decide whether it can safely select, create a provider history snapshot, restore, or sync work.

- A2. **Human developer.** Reads status, review, and provider history snapshot information in the TUI or app-server clients and expects provider-native concepts to remain understandable.

- A3. **Version control provider extension.** Owns detection, status, diffing, selection, provider history snapshot creation, restore, line switching, sync, and provider-specific operations for one VCS family.

- A4. **App-server and SDK clients.** Need a stable protocol that works across providers without hardcoding git-specific method names or assumptions.

- A5. **Downstream distribution author.** May include git, replace git with another provider, or ship multiple providers with deterministic selection rules.

---

## Requirements

**Provider Model**

- R1. Roder must define version control as a replaceable provider subsystem, with a stable native API in `roder-api` or the equivalent extension-facing crate.

- R2. A VCS provider must report its provider identity, display name, detected workspace root, active line of work, and capability set for the current workspace.

- R3. Provider detection must support multiple installed providers and produce a deterministic active provider for a workspace.

- R4. If no provider claims a workspace, Roder must degrade gracefully: status and review surfaces show that version control is unavailable, while unrelated file, shell, and agent workflows continue.

- R5. The bundled git implementation must be installed through the same provider mechanism as other VCS providers.

**Canonical Workflow Operations**

- R6. Roder must provide a canonical status operation that describes changed files, repository/workspace root, active line of work, base or review target when known, and provider capability data.

- R7. Roder must provide a canonical changed-file listing that covers modified, added, deleted, renamed, untracked, and provider-native changed states where representable.

- R8. Roder must provide a canonical changed-content read operation that returns bounded diff or patch text for a selected changed item, while allowing providers to mark content as binary, unavailable, or provider-native.

- R9. Roder must provide a canonical change-selection operation for snapshot workflows, with capability distinctions for no selection, path selection, hunk selection, and provider-native selection.

- R10. Roder must provide a canonical snapshot operation for creating a durable user-visible point in provider history, mapping to git commit, jj change description/finalization behavior, SVN commit, or a provider-specific equivalent.

- R11. Roder must provide a canonical restore operation for discarding or restoring selected work where supported by the provider.

- R12. Roder must provide canonical line-of-work listing and switching where the provider has a branch, bookmark, working-copy, or equivalent concept.

- R13. Roder must provide canonical sync intent operations for provider-supported fetch, pull/update, push, or equivalent upstream interactions.

**Capability Semantics**

- R14. Every canonical operation must have explicit capability metadata that distinguishes supported, unsupported, partially supported, and provider-native behavior.

- R15. Capability metadata must be available to the agent runtime, app-server clients, TUI status/review surfaces, and generated SDK consumers.

- R16. Unsupported operations must fail with structured provider-aware errors that explain the missing capability without implying the provider is broken.

- R17. Provider-native extras must be namespaced by provider and discoverable without polluting the canonical VCS model.

**User-Facing Workflows**

- R18. Existing git review surfaces should become provider-neutral VCS review surfaces that render provider identity and line-of-work information alongside changed files and diffs.

- R19. Workspace-change observation should become version-control reconciled rather than git-reconciled, while retaining enough provenance to know which provider produced the observation.

- R20. The built-in commit workflow should become VCS-aware. In git workspaces it may still present commit language, but the canonical workflow should not assume every provider has git-style staging or commit semantics.

- R21. TUI and app-server status should render VCS provider state generically, including provider identity and active line of work.

- R22. Skills and commands that depend on version control must declare VCS needs in provider-neutral terms, with git-specific instructions scoped to the bundled git provider.

**Protocol and Compatibility Direction**

- R23. New app-server methods and generated schemas should use `vcs/*` naming for canonical version-control capabilities.

- R24. Existing `git/*` protocol concepts should be replaced by the canonical `vcs/*` surface as part of this feature, rather than expanded into a second long-lived API family.

- R25. Documentation must identify git as the included provider and explain how other providers can implement the same canonical workflows plus namespaced extras.

---

## Key Flows

- F1. **Provider detection**
  - **Trigger:** A workspace-bound runtime, TUI, or app-server method needs VCS context.
  - **Actors:** A1, A3, A4
  - **Steps:** Installed providers inspect the workspace, report whether they claim it, and Roder selects one active provider using deterministic precedence.
  - **Outcome:** The caller receives active provider metadata and capabilities, or a clear unavailable state.
  - **Covered by:** R2, R3, R4, R5

- F2. **Review current changes**
  - **Trigger:** A user or client opens the review surface, or an agent needs live change context.
  - **Actors:** A1, A2, A3, A4
  - **Steps:** Roder asks the active provider for status and changed files, then reads bounded changed-content pages on demand.
  - **Outcome:** Review surfaces work in git, jj, SVN, and future provider workspaces according to provider capabilities.
  - **Covered by:** R6, R7, R8, R18, R21

- F3. **Create a provider history snapshot**
  - **Trigger:** A user asks the agent to commit, create a VCS snapshot, or otherwise save a scoped slice of work.
  - **Actors:** A1, A2, A3
  - **Steps:** Roder checks provider selection capabilities, prepares the requested scope if supported, creates the provider-specific durable history snapshot, and reports the resulting identity.
  - **Outcome:** Git repositories can commit scoped work; non-git providers can perform their closest safe equivalent without pretending to support git staging.
  - **Covered by:** R9, R10, R14, R16, R20, R22

- F4. **Restore work**
  - **Trigger:** A user or agent asks to discard or restore selected changes.
  - **Actors:** A1, A2, A3
  - **Steps:** Roder validates whether the provider supports the requested restore granularity, performs the restore if supported, and reports what changed.
  - **Outcome:** Restore behavior is explicit about unsupported hunk, path, binary, or provider-native cases.
  - **Covered by:** R11, R14, R16

- F5. **Switch or sync line of work**
  - **Trigger:** A user asks to change branches/bookmarks/working copies or interact with an upstream authority.
  - **Actors:** A1, A2, A3
  - **Steps:** Roder lists provider-supported line or sync actions, executes the requested supported operation, and refreshes VCS status.
  - **Outcome:** The workflow supports git branch/push/pull, jj bookmark or equivalent operations, SVN update/commit-style flows, and future provider mappings where available.
  - **Covered by:** R12, R13, R14, R21

---

## Acceptance Examples

- AE1. **Git remains the default happy path**
  - **Covers:** R5, R6, R7, R8, R10, R18, R20
  - **Given:** A workspace is a normal git repository with modified and untracked files.
  - **When:** A client requests VCS status and reads a changed file.
  - **Then:** Roder reports the git provider, active branch, changed files, and bounded diff content through `vcs/*` semantics.

- AE2. **A jj workspace does not need to mimic git staging**
  - **Covers:** R9, R10, R14, R16, R17, R20
  - **Given:** A workspace is claimed by a jj provider that does not expose git-style hunk staging.
  - **When:** The snapshot workflow requests hunk selection.
  - **Then:** Roder reports that hunk selection is unsupported or provider-native, and the workflow either uses a supported selection mode or stops with a structured capability error.

- AE3. **An SVN workspace can review and sync without branches**
  - **Covers:** R6, R12, R13, R14, R21
  - **Given:** A workspace is claimed by an SVN provider with no local branch concept.
  - **When:** Status and sync capabilities are requested.
  - **Then:** Roder reports the active provider and working-copy identity, omits or marks line switching unsupported, and exposes update/commit-compatible sync capabilities where supported.

- AE4. **No VCS provider is not fatal**
  - **Covers:** R4, R16, R21
  - **Given:** A workspace has no recognized VCS metadata.
  - **When:** The TUI status line and review surface load.
  - **Then:** They show VCS unavailable without breaking file browsing, shell execution, or agent turns.

- AE5. **Provider-native extras stay namespaced**
  - **Covers:** R17, R25
  - **Given:** A jj provider exposes an operation-log inspection action that has no canonical VCS equivalent.
  - **When:** Clients list provider extras.
  - **Then:** The extra appears under a jj namespace and does not add jj-specific fields to canonical status or snapshot requirements.

---

## Scope Boundaries

### Deferred for later

- Remote hosting workflows such as GitHub pull request creation, GitLab merge request handling, code review comments, and issue linking.

- Rich provider-specific UI for every native concept beyond discoverable namespaced extras.

- Process-isolated or WASM VCS providers. Native extension capability modeling should leave room for this later, but the first provider mechanism can stay in-process.

- Cross-repository orchestration where a single user task spans multiple unrelated VCS roots.

### Outside this feature's identity

- A universal staging model. Roder should support change selection, but it should not force all providers into git's index semantics.

- A universal branch model. Roder should expose line-of-work concepts where available, but providers may map that to branches, bookmarks, working copies, revisions, or unsupported.

- Replacing provider-native command-line tools for all advanced workflows. The canonical surface covers agent-critical workflows; advanced provider behavior can remain namespaced.

---

## Dependencies / Assumptions

- The extension architecture continues to treat `roder-api` as the stable native API for extension authors.

- App-server methods, protocol schemas, SDK codegen, TUI status/review surfaces, runtime workspace-change observation, and built-in skills can be updated together as a breaking surface change.

- Git should remain available in the default distribution, but no core workflow should require git specifically once this feature lands.

- Provider detection precedence must be explicit enough to handle nested or overlapping metadata, such as a jj workspace that also uses git storage internally.

---

## Sources / Research

- `crates/roder-app-server/src/git_changes.rs` currently implements git-specific live change listing and patch reading.

- `crates/roder-core/src/workspace_changes.rs` currently captures git-reconciled workspace changes around shell and exec tool calls.

- `schemas/app-server/roder-app-server.v1.json` currently exposes `git/changes/list` and `git/changes/read` as stable app-server methods.

- `docs/app-server/api.md` documents git review methods, workspace observed changes, and the built-in commit skill surface.

- `roadmap/foundations/roder_extensibility_foundations_extensions.md` defines the provider/contributor split, registry-based extension installation, capability modeling, and the rule that extensions depend on stable API rather than core internals.
