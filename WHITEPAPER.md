# Roder: A Rust-Native Extensible Harness for Coding Agents

## A technical mission paper

### Working subtitle

**The last agent harness: a stable, extensible foundation for coding agents, research systems, reinforcement-learning environments, and AI-native developer tools.**

---

# 1. Abstract

Roder is a Rust-native, open-source agent harness designed to become the common foundation for coding agents, AI research systems, reinforcement-learning environments, and customer-facing AI developer tools.

Most agent systems today are rebuilt repeatedly around the same core primitives: model invocation, tool execution, filesystem access, terminal mediation, sandboxing, context construction, session persistence, approval policy, event streaming, and replay. These systems are often written in Python or Node.js because those ecosystems offer fast iteration and flexible extension. That flexibility is useful, but it frequently comes at the cost of portability, performance, reliability, packaging discipline, and long-term architectural stability.

Roder aims to provide a different foundation: a highly portable, high-performance, strongly typed Rust harness that can be extended at the harness level without requiring forks. The project is not intended to be just another coding agent. It is intended to be the substrate upon which coding agents, research harnesses, training loops, internal lab systems, and end-user developer applications can be built.

The central premise is simple:

> The AI ecosystem should not need to rewrite the same agent harness every time a new model, product, research direction, or interaction paradigm appears.

Roder seeks to become a reference implementation of the core machinery required by modern coding agents: inference abstraction, tool orchestration, context assembly, sandboxed execution, stateful sessions, resumable transcripts, event-sourced traces, policy enforcement, and extensibility boundaries.

The long-term ambition is comparable in spirit to the Linux kernel: a durable, well-maintained, extensible core that many different systems can build upon. Roder remains forkable, but its design goal is to make forking unnecessary for most serious customization. New inference technologies, storage systems, context engines, safety policies, UI layers, training environments, and product-specific behavior should be expressible as extensions, providers, plugins, or downstream distributions rather than hard forks.

Roder is not bound to any individual model provider, protocol, product philosophy, or user interface. It is a harness before it is an app. It is a box of well-designed primitives for building serious AI systems.

---

# 2. The problem: every agent stack keeps rebuilding the same harness

The current coding-agent ecosystem has a recurring pattern.

A team starts by building a model loop. The model receives a prompt, chooses a tool call, runs a command, reads files, edits code, and returns a response. Initially this seems straightforward. But as soon as the system matures, the team needs more:

* robust filesystem mediation;
* command execution and sandboxing;
* model-provider abstraction;
* tool schemas and tool routing;
* event streaming;
* logs and replay;
* approval policy;
* resumable sessions;
* multi-turn context management;
* prompt assembly;
* memory and indexing;
* workspace-scoped capabilities;
* cancellation and interruption;
* terminal user experience;
* app-server control plane;
* telemetry;
* security boundaries;
* benchmarks and training traces;
* extension points;
* packaging and distribution.

At that point, the project is no longer just a wrapper around an LLM. It has become an operating environment for an agent.

Many teams end up building this operating environment from scratch. Some do so in Python because Python is familiar in research. Others use Node.js because it integrates easily with developer tooling and web interfaces. These languages are excellent for fast iteration, but they can be less ideal as the foundation for a portable, durable, high-performance, security-sensitive local agent harness.

The result is fragmentation. Each system develops its own internal representation of messages, tools, sessions, files, actions, approvals, traces, and state. Each system separately solves persistence, replay, sandboxing, provider integration, and extension. Useful ideas are trapped inside individual products or research codebases. Integrations become bespoke. Labs fork harnesses because the upstream architecture cannot express the modifications they need.

Roder exists to break that cycle.

---

# 3. Mission

Roder's mission is to provide a stable, extensible, Rust-native harness for building coding agents and agentic developer systems.

Roder should be suitable for:

* AI labs building research agents;
* model creators evaluating tool-use behavior;
* reinforcement-learning teams constructing coding environments;
* companies building customer-facing coding assistants;
* open-source agent projects needing a serious runtime foundation;
* internal platform teams standardizing agent execution;
* researchers experimenting with context, memory, inference, and policy;
* developers building terminal, IDE, web, or headless agent interfaces.

Roder should not be tightly coupled to one model provider, one API shape, one UI, one product strategy, one storage backend, or one extension style.

The guiding statement is:

> Roder is the core harness. Everything else is a distribution, extension, provider, policy, interface, or experiment.

A mature Roder ecosystem should make it possible for a project like a terminal coding assistant, an IDE-integrated agent, an RL training harness, or a cloud-hosted coding environment to share the same underlying runtime instead of independently rebuilding it.

---

# 4. Why Rust

Roder is written in Rust because the harness itself should be reliable infrastructure.

Rust offers:

* native performance;
* strong compile-time guarantees;
* memory safety without a garbage collector;
* excellent packaging through Cargo;
* predictable single-binary distribution;
* strong async/networking ecosystem;
* high-quality serialization and schema tooling;
* good foundations for sandboxing and systems integration;
* a credible path to WASM hosting and component-based extension;
* a culture of explicitness that matches security-sensitive agent systems.

The goal is not to make Roder less flexible than Python or Node.js systems. The goal is to put flexibility behind explicit, typed boundaries.

Roder should allow rapid experimentation, but not by encouraging unstructured mutation of global state. It should allow deep extension, but not by exposing unstable internals. It should allow researchers to replace critical pieces, but through designed provider interfaces and canonical runtime representations.

Rust is particularly well-suited to this philosophy because it encourages the separation of stable interfaces from implementation detail.

---

# 5. Roder is a harness, not just an agent

A coding agent is a behavior. A harness is the environment that makes that behavior possible.

Roder is not primarily a model prompt, a personality, or a product UI. Roder is the machinery beneath such systems.

A harness is responsible for:

* receiving user or training-environment input;
* assembling context;
* selecting and invoking an inference engine;
* exposing tools safely;
* mediating filesystem and process access;
* recording what happened;
* enforcing policy;
* supporting interruption;
* persisting state;
* enabling replay and resume;
* abstracting model providers;
* allowing extensions to contribute or replace behavior.

This distinction matters.

If Roder were only a coding assistant, its architecture would naturally optimize for one end-user experience. But Roder's goal is broader: it should be possible for many coding assistants, research systems, and training platforms to build on top of it.

Roder should therefore prioritize stable primitives over product-specific behavior.

---

# 6. Design philosophy

## 6.1 Stable core, extensible edges

The core should own invariants:

* lifecycle ordering;
* cancellation semantics;
* permission enforcement;
* event ordering;
* session identity;
* transcript semantics;
* tool-call routing;
* capability checks;
* state migration boundaries.

Extensions should be able to provide behavior, but not corrupt the runtime.

This mirrors the kernel-like philosophy: subsystems can be replaced, extended, or configured, but they operate within a stable contract.

## 6.2 Providers, not forks

A lab should not need to fork Roder to add:

* a new inference API;
* a new model wire format;
* a new context retrieval engine;
* a new session store;
* a new checkpoint format;
* a new policy evaluator;
* a new sandbox backend;
* a new telemetry sink;
* a new memory engine;
* a new training environment bridge.

These should be native provider interfaces.

## 6.3 Canonical internal representations

The harness must not leak provider-specific formats into the core.

There should be canonical Roder representations for:

* conversations;
* turns;
* messages;
* reasoning events;
* tool calls;
* tool results;
* file changes;
* context blocks;
* inference requests;
* inference stream events;
* session records;
* checkpoints;
* permissions;
* approvals.

Providers translate to and from these representations.

This is what allows Roder to support the Responses API, Chat Completions-style APIs, Anthropic Messages-style APIs, local inference runtimes, and future unimagined systems without rewriting the core harness.

## 6.4 Capability-based access

Agent systems interact with sensitive local resources: files, shells, secrets, networks, repositories, credentials, and user workspaces.

Roder should treat access as explicit capability grants, not ambient authority.

Extensions should not receive raw filesystem or process access. They should receive scoped handles:

* `ScopedFilesystem`;
* `ScopedProcessRunner`;
* `ScopedNetwork`;
* `ScopedSecrets`;
* `ApprovalClient`;
* `EventSink`.

This ensures that even native extensions operate within declared boundaries.

## 6.5 Event-sourced by default

A serious agent harness should be replayable.

Roder should model execution as a stream of structured events:

* session created;
* thread started;
* user input received;
* context assembled;
* model request sent;
* model event received;
* tool call requested;
* approval requested;
* command executed;
* file changed;
* turn item appended;
* turn completed;
* extension state checkpointed.

Event-sourced execution supports:

* debugging;
* reproducibility;
* crash recovery;
* auditability;
* reinforcement-learning traces;
* offline analysis;
* UI synchronization;
* deterministic replay where possible.

## 6.6 UI is a client, not the core

Roder may ship with a TUI, but the TUI should not be the runtime.

The runtime should expose a local control plane. The TUI, CLI, IDE integration, headless executor, test runner, web UI, or training environment should all speak to the same core through a stable protocol.

This prevents UI decisions from contaminating the harness.

---

# 7. Conceptual architecture

At a high level, Roder can be understood as the following layers:

```text
Applications
  - TUI
  - CLI
  - IDE integration
  - web frontend
  - RL environment
  - customer-facing product
  - test harness

Control Plane
  - local app server
  - session API
  - event subscription
  - tool invocation API
  - extension introspection
  - policy and approval API

Roder Core Runtime
  - thread and turn lifecycle
  - agent loop
  - cancellation
  - permission enforcement
  - event bus
  - tool router
  - context orchestration
  - inference orchestration
  - persistence orchestration

Native Extension Kernel
  - inference engines
  - wire dialects
  - context providers
  - context planners
  - session stores
  - checkpoint stores
  - policy contributors
  - sandbox backends
  - memory stores
  - event sinks

Execution Substrate
  - filesystem broker
  - process broker
  - network broker
  - secret broker
  - sandbox backend
  - model transports
  - storage backends
```

The central design constraint is dependency direction:

```text
extensions -> roder-api
core       -> roder-api
apps       -> roder-protocol
roder-cli  -> core + selected extensions
```

Extensions should not depend on unstable internals. The core should not depend on downstream extensions. A distribution binary composes the core with selected extensions.

---

# 8. Native extension kernel

The native extension kernel is the heart of Roder's long-term extensibility.

Rather than treating extensions as scripts or user-facing workflows, Roder treats native extensions as first-class subsystem providers and contributors.

A native extension can provide:

* an inference engine;
* a wire dialect;
* a context provider;
* a context planner;
* a session store;
* a checkpoint store;
* a memory backend;
* a policy contributor;
* a sandbox backend;
* an event sink;
* a state codec;
* a model catalog;
* a tool executor;
* a lifecycle contributor.

The basic shape:

```rust
pub trait RoderExtension: Send + Sync + 'static {
    fn manifest(&self) -> ExtensionManifest;

    fn install(
        &self,
        registry: &mut ExtensionRegistryBuilder,
    ) -> anyhow::Result<()>;
}
```

A lab distribution can compile in its extensions:

```rust
fn main() -> anyhow::Result<()> {
    roder_cli::run(|registry| {
        registry.install(roder_ext_responses::extension())?;
        registry.install(roder_ext_anthropic::extension())?;
        registry.install(lab_ext_context::extension())?;
        registry.install(lab_ext_storage::extension())?;
        Ok(())
    })
}
```

No fork is required. The lab builds a distribution.

---

# 9. Inference extensibility

Roder must not be bound to one inference provider or one wire format.

Modern model APIs differ across:

* message representation;
* tool-call semantics;
* streaming format;
* reasoning visibility;
* multimodal support;
* structured output support;
* prompt caching;
* tool-result attachment;
* context-window behavior;
* system/developer/user instruction hierarchy;
* retry semantics;
* rate-limit metadata;
* usage accounting.

Roder should not choose one provider's format as the internal truth.

Instead, Roder defines a canonical inference request and canonical inference event stream.

```rust
pub struct AgentInferenceRequest {
    pub model: ModelSelection,
    pub instructions: InstructionBundle,
    pub conversation: Vec<ConversationItem>,
    pub tools: Vec<ToolSpec>,
    pub tool_choice: ToolChoice,
    pub reasoning: ReasoningConfig,
    pub output: OutputConfig,
    pub runtime: RuntimeHints,
}
```

Providers implement:

```rust
#[async_trait::async_trait]
pub trait InferenceEngine: Send + Sync + 'static {
    fn id(&self) -> InferenceEngineId;

    fn capabilities(&self) -> InferenceCapabilities;

    async fn list_models(
        &self,
        ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>>;

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream>;
}
```

The stream returns canonical events:

```rust
pub enum InferenceEvent {
    MessageDelta(MessageDelta),
    ReasoningDelta(ReasoningDelta),
    ToolCallStarted(ToolCallStarted),
    ToolCallDelta(ToolCallDelta),
    ToolCallCompleted(ToolCallCompleted),
    Usage(TokenUsage),
    Completed(CompletionMetadata),
    Failed(InferenceFailure),
}
```

This allows Roder to support:

* Responses-style APIs;
* Chat Completions-style APIs;
* Anthropic Messages-style APIs;
* local inference engines;
* research inference runtimes;
* future model protocols that do not exist yet.

The core harness does not care which provider produced the events. It only consumes canonical Roder events.

---

# 10. Wire dialects

Inference engines may combine transport, authentication, model discovery, request construction, streaming, and decoding. But Roder should also support separating transport from wire format.

A `WireDialect` translates canonical Roder inference requests to a provider-specific representation and translates provider-specific stream events back into canonical Roder events.

```rust
pub trait WireDialect: Send + Sync + 'static {
    fn id(&self) -> WireDialectId;

    fn encode_request(
        &self,
        request: &AgentInferenceRequest,
    ) -> anyhow::Result<OutboundRequest>;

    fn decode_stream_event(
        &self,
        event: ProviderStreamEvent,
    ) -> anyhow::Result<Vec<InferenceEvent>>;
}
```

This separation allows composition:

```text
HTTP transport + Responses dialect
HTTP transport + Chat Completions dialect
HTTP transport + Anthropic Messages dialect
Local IPC transport + custom lab dialect
In-process engine + no external wire dialect
```

The dialect system is what makes Roder future-facing. If a new provider invents a better representation for agentic reasoning, Roder should not need to change its core loop. A new dialect or engine can translate between that representation and Roder's canonical event model.

---

# 11. Context internals

Context is not just prompt text.

In a serious coding agent, context may include:

* repository instructions;
* current task state;
* selected files;
* git metadata;
* issue metadata;
* test results;
* search results;
* memory retrieval;
* dependency graphs;
* code ownership;
* policy constraints;
* previous turn summaries;
* tool availability;
* environment information;
* active plans;
* hidden control instructions;
* evaluation harness metadata.

Roder should treat context assembly as a structured subsystem.

A simple extension can contribute context blocks:

```rust
#[async_trait::async_trait]
pub trait ContextProvider: Send + Sync {
    fn id(&self) -> ContextProviderId;

    async fn resolve(
        &self,
        ctx: ContextResolutionContext<'_>,
        query: ContextQuery,
    ) -> anyhow::Result<Vec<ContextBlock>>;
}
```

Context blocks are typed:

```rust
pub enum ContextBlock {
    Instruction(InstructionBlock),
    RepositoryFact(RepositoryFact),
    RetrievedDocument(RetrievedDocument),
    Memory(MemoryBlock),
    ToolAvailability(ToolAvailabilityBlock),
    Environment(EnvironmentBlock),
    SafetyPolicy(SafetyPolicyBlock),
}
```

More advanced extensions can replace planning:

```rust
pub trait ContextPlanner: Send + Sync {
    fn plan(
        &self,
        ctx: &ThreadContext,
        input: &UserInput,
        budget: ContextBudget,
    ) -> anyhow::Result<ContextPlan>;
}
```

This lets labs experiment with:

* different context-window allocation strategies;
* provider-specific prompt shaping;
* retrieval-augmented coding context;
* memory systems;
* repository-level graph context;
* instruction hierarchies;
* long-running task state;
* compression and compaction;
* training-time context perturbation.

The host runtime remains stable. Context internals become pluggable.

---

# 12. Session persistence, resume, and replay

Roder should treat persistence as a core harness surface, not an implementation detail.

Coding agents need to resume work. Research systems need replayable traces. Training systems need structured trajectories. Product systems need user-visible history and auditability.

Roder should support multiple persistence strategies:

* in-memory ephemeral sessions;
* local JSONL traces;
* SQLite-backed local sessions;
* encrypted local storage;
* git-backed run directories;
* remote stores;
* research replay stores;
* immutable training trajectories.

The interface:

```rust
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync + 'static {
    async fn create_thread(
        &self,
        req: CreateThreadRequest,
    ) -> anyhow::Result<ThreadRecord>;

    async fn load_thread(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<ThreadSnapshot>;

    async fn append_turn_item(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        item: TurnItem,
    ) -> anyhow::Result<()>;

    async fn complete_turn(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        summary: TurnCompletion,
    ) -> anyhow::Result<()>;

    async fn fork_thread(
        &self,
        source: ThreadId,
        options: ForkOptions,
    ) -> anyhow::Result<ThreadRecord>;

    async fn list_threads(
        &self,
        query: ThreadListQuery,
    ) -> anyhow::Result<Page<ThreadRecord>>;
}
```

Roder should also support checkpoint stores:

```rust
#[async_trait::async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn append_event(
        &self,
        event: RuntimeEvent,
    ) -> anyhow::Result<EventOffset>;

    async fn load_events(
        &self,
        thread_id: ThreadId,
        from: Option<EventOffset>,
    ) -> anyhow::Result<Vec<RuntimeEvent>>;

    async fn save_snapshot(
        &self,
        thread_id: ThreadId,
        snapshot: RuntimeSnapshot,
    ) -> anyhow::Result<()>;

    async fn load_latest_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<Option<RuntimeSnapshot>>;
}
```

This opens the door to serious research and product use:

* "resume this coding session";
* "fork from turn 12 and try another strategy";
* "replay this trajectory against a different model";
* "export this run as training data";
* "audit every command and file edit";
* "compare provider behavior on the same context."

Roder should make these operations first-class.

---

# 13. Extension-owned state and migrations

Native extensions will need private state.

A context extension may cache repository metadata. An inference extension may track provider-specific continuation state. A storage extension may maintain indexes. A policy extension may track approvals. A memory extension may maintain embeddings or summaries.

The host should not need to understand these internals, but it must be able to persist and restore them.

Roder should provide scoped extension-owned state:

```rust
pub enum ExtensionStoreScope {
    Process,
    Session,
    Thread,
    Turn,
    ToolCall,
}
```

And extension-controlled codecs:

```rust
pub trait ExtensionStateCodec: Send + Sync {
    fn extension_id(&self) -> ExtensionId;

    fn scope(&self) -> ExtensionStoreScope;

    fn schema_version(&self) -> u32;

    fn serialize(
        &self,
        store: &ExtensionData,
    ) -> anyhow::Result<Option<Vec<u8>>>;

    fn deserialize(
        &self,
        bytes: &[u8],
        store: &ExtensionData,
    ) -> anyhow::Result<()>;

    fn migrate(
        &self,
        from_version: u32,
        bytes: &[u8],
    ) -> anyhow::Result<Vec<u8>>;
}
```

This creates a clean division:

* Roder owns lifecycle and persistence hooks.
* Extensions own their private schema.
* Extensions own migrations for their state.
* The host stores opaque extension snapshots.

This is essential for long-term compatibility.

---

# 14. Tools and execution

Although Roder is more than a tool runner, tool execution remains central.

A coding agent needs to:

* read files;
* search repositories;
* edit files;
* run commands;
* execute tests;
* inspect diffs;
* interact with language servers;
* call external systems;
* operate inside constrained workspaces.

Roder should expose tools through a unified interface:

```rust
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    fn spec(&self) -> ToolSpec;

    async fn call(
        &self,
        ctx: ToolExecutionContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolResult, ToolError>;
}
```

Tool input and output may remain schema-described JSON because model-facing tool schemas are dynamic. But the execution context must be strongly typed and capability-scoped:

```rust
pub struct ToolExecutionContext<'a> {
    pub fs: &'a ScopedFilesystem,
    pub process: &'a ScopedProcessRunner,
    pub network: &'a ScopedNetwork,
    pub secrets: &'a ScopedSecrets,
    pub approval: &'a ApprovalClient,
    pub events: &'a EventSink,
    pub cancellation: CancellationToken,
}
```

This makes tools portable across inference providers. The model-facing tool schema can vary, but the runtime execution semantics stay stable.

---

# 15. Policy and safety

A local coding harness touches real systems. Roder should treat safety as an architectural primitive.

Policy should be layered:

```text
Technical sandbox
  What is technically possible?

Capability model
  What has this extension/tool/session been granted?

Approval policy
  What must ask the user before proceeding?

Policy contributors
  What do org/lab/project rules allow or deny?

Audit log
  What happened, and why was it allowed?
```

Native policy contributors should be able to review sensitive actions:

```rust
#[async_trait::async_trait]
pub trait PolicyContributor: Send + Sync {
    async fn review(
        &self,
        ctx: PolicyContext<'_>,
        request: PolicyRequest,
    ) -> anyhow::Result<Option<PolicyDecision>>;
}
```

Decision resolution should be conservative:

```text
deny > require approval > allow > abstain
```

No extension should be able to silently bypass a stricter policy.

Roder's policy model should support:

* personal local use;
* enterprise restrictions;
* research sandboxing;
* training environment constraints;
* CI-like automated runs;
* fully headless operation with strict preconfigured permissions.

---

# 16. App server as control plane

Roder should ship with a local app server embedded in the binary.

The app server is the control plane for:

* creating sessions;
* starting turns;
* streaming events;
* interrupting execution;
* listing tools;
* inspecting extensions;
* choosing providers;
* reading transcripts;
* managing approvals;
* controlling persistence;
* integrating with UIs.

The TUI should be a client of the app server. So should an IDE integration, a web frontend, a test harness, or an RL environment.

This architecture prevents the terminal UI from becoming the runtime.

The app server protocol may use JSON-RPC, protobuf, gRPC, Connect, or multiple transports. The important point is that Roder has a stable control-plane protocol independent of UI.

A possible split:

```text
Rust native API
  For trusted compile-time extensions.

App server protocol
  For clients and controllers.

Process extension protocol
  For non-native runtime extensions.

MCP bridge
  For external tool ecosystem compatibility.
```

Roder should not bet the entire system on one extension mechanism.

---

# 17. Strong typing and dynamic extensibility

Roder should be strongly typed where the domain is stable and dynamic where extensibility requires runtime discovery.

Strongly typed:

* sessions;
* turns;
* lifecycle;
* inference events;
* capability requests;
* approvals;
* filesystem operations;
* process execution;
* storage records;
* checkpoints;
* extension manifests;
* provider capabilities.

Dynamic or schema-described:

* model-facing tool inputs;
* model-facing tool outputs;
* extension-specific metadata;
* unknown future content blocks;
* provider-specific extras;
* research-only experimental fields.

The core should avoid both extremes:

* not everything should be `serde_json::Value`;
* not every model-facing tool schema should require a Rust type known at compile time.

The correct approach is typed envelopes with extensible payloads.

---

# 18. Roder as an RL harness

Roder should be useful not only for interactive coding agents, but also for reinforcement learning and evaluation.

An RL-oriented harness needs:

* deterministic environment setup where possible;
* task definitions;
* workspace snapshots;
* action traces;
* tool-call logs;
* model request/response records;
* reward hooks;
* evaluators;
* resumable trajectories;
* branching and replay;
* provider swapping;
* perturbation experiments;
* run metadata;
* exportable datasets.

Roder's event-sourced architecture naturally supports this.

A training system could run Roder headlessly:

```text
load task
create workspace
start thread
feed task prompt
stream agent events
execute tools in sandbox
evaluate result
persist trajectory
export run artifact
```

Different labs could provide extensions for:

* reward models;
* task loaders;
* environment resetters;
* benchmark adapters;
* trace exporters;
* model backends;
* context perturbation;
* custom tool policies.

The same harness primitives used by an end-user coding agent become useful for training and evaluation.

---

# 19. Relationship to existing tools

Roder is not trying to be a clone of any specific coding agent.

It is also not trying to replace every higher-level product.

Instead, Roder aims to be the substrate on which such products can be built.

A user-focused coding agent can build on Roder and provide its own UX, defaults, marketplace, and product opinion. A research lab can build on Roder and provide its own inference engines, storage, and evaluation environment. A company can build on Roder and provide its own policies, authentication, context sources, and deployment model.

The difference is that they should not need to reimplement the harness.

Roder should be lower-level than a polished end-user application, but higher-level than a bag of unrelated crates.

It should be the maintained, coherent, reference-quality foundation.

---

# 20. Governance philosophy

Roder should be open source and forkable, but designed so that forking is rarely necessary.

The project should encourage:

* stable extension APIs;
* RFC-driven major changes;
* experimental feature gates;
* provider crates outside the core;
* upstreaming of common abstractions;
* downstream distributions;
* compatibility tests for extension APIs;
* clear deprecation policy;
* long-term maintainability over novelty.

A Linux-kernel-like analogy is useful, but it should be applied carefully.

Roder should not copy kernel governance literally. But it should learn from the underlying principle:

> A stable extensible core allows an ecosystem to build around it without fragmenting into incompatible forks.

The core should remain strict about invariants and conservative about API stability. Experimental ideas should live in extensions until they prove general enough for core.

---

# 21. Project structure

A plausible crate layout:

```text
roder-api
  Stable native extension traits and canonical types.

roder-core
  Agent loop, lifecycle, cancellation, event ordering, permission enforcement.

roder-inference
  InferenceEngine, WireDialect, model registry, canonical inference IR.

roder-context
  ContextProvider, ContextPlanner, context budgets, context blocks.

roder-session
  SessionStore, CheckpointStore, transcript model, replay primitives.

roder-tools
  ToolSpec, ToolExecutor, tool routing, MCP bridge types.

roder-sandbox
  Filesystem/process/network/secret brokers and sandbox backends.

roder-protocol
  App-server protocol, schemas, generated clients.

roder-app-server
  Embedded local control plane.

roder-extension-host
  Native extension installation and provider selection.

roder-tui
  Reference terminal UI client.

roder-cli
  Command entrypoint and distribution wiring.

roder-ext-responses
  Responses-style inference engine.

roder-ext-chat-completions
  Chat-completions-style inference engine.

roder-ext-anthropic
  Anthropic-messages-style inference engine.

roder-ext-sqlite-session
  SQLite session and checkpoint storage.

roder-ext-jsonl-replay
  JSONL event/replay storage.
```

The exact crate names can change, but the separation matters.

---

# 22. MVP strategy

Roder should not attempt to implement every extension point immediately.

A sensible MVP would include:

## Phase 1: Core runtime

* thread/turn lifecycle;
* canonical turn items;
* event bus;
* cancellation;
* tool execution;
* filesystem/process brokers;
* basic TUI or CLI;
* one inference provider;
* one local session store.

## Phase 2: Native extension kernel

* `RoderExtension` trait;
* `ExtensionRegistryBuilder`;
* inference engine registration;
* context provider registration;
* tool contributor registration;
* policy contributor registration;
* session store registration;
* extension manifest introspection.

## Phase 3: Provider plurality

* Responses-style provider;
* Chat-Completions-style provider;
* Anthropic-style provider;
* local/mock provider for testing;
* provider capability negotiation.

## Phase 4: Persistence and replay

* event log;
* checkpoint store;
* resume sessions;
* fork sessions;
* export traces;
* replay against another provider.

## Phase 5: App server

* stable local control plane;
* event subscription;
* TUI as client;
* headless execution;
* IDE/web integration path.

## Phase 6: Research and RL surfaces

* task loaders;
* reward/evaluator hooks;
* trace export;
* benchmark adapters;
* environment reset/checkpoint integration.

## Phase 7: Process/WASM extensions

* non-native extension protocol;
* WASM component experiments;
* sandboxed extension runtime;
* broader ecosystem compatibility.

The native extension design should be present early, even if not all extension types are implemented at once.

---

# 23. What Roder should not become

Roder should avoid several traps.

## 23.1 Not a monolith

Roder should not turn into one giant application where every feature enters core.

Core should provide primitives and invariants. Specialized behavior should live in extensions.

## 23.2 Not a provider wrapper

Roder should not be a wrapper around one model API.

Provider-specific logic belongs in provider extensions. The harness should operate over canonical representations.

## 23.3 Not a prompt collection

Skills and prompts are useful, but they are not the core harness.

Roder's main value is the execution substrate, not a set of instructions.

## 23.4 Not a UI-first product

A polished TUI may be important, but the TUI must not define the architecture.

Roder should be usable headlessly, inside IDEs, in CI, in training loops, and behind other products.

## 23.5 Not unsafe extensibility

Extensibility should not mean arbitrary mutation of internal state.

Extensions should contribute through typed, capability-scoped interfaces.

---

# 24. A concrete example: adding a new inference technology

Imagine a new lab invents an inference runtime called GraphReason.

GraphReason is not chat-based. It accepts a graph of tasks, context nodes, tool affordances, and execution constraints. It streams graph mutations rather than text deltas.

Without Roder, integrating GraphReason into a coding agent might require building a new harness.

With Roder, the lab writes:

```rust
pub struct GraphReasonExtension;

impl RoderExtension for GraphReasonExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest::new("com.lab.graphreason", "GraphReason")
    }

    fn install(
        &self,
        registry: &mut ExtensionRegistryBuilder,
    ) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(GraphReasonEngine::new()));
        registry.context_planner(Arc::new(GraphReasonContextPlanner::new()));
        Ok(())
    }
}
```

The engine translates Roder's canonical request into GraphReason's graph format. It translates GraphReason's stream back into Roder inference events.

The existing Roder runtime still handles:

* sessions;
* tools;
* filesystem;
* approvals;
* checkpointing;
* UI events;
* replay;
* storage;
* cancellation;
* policy.

That is the power of harness-level extensibility.

---

# 25. A concrete example: replacing persistence

A customer-facing app may want encrypted local SQLite storage.

A research lab may want immutable JSONL traces.

An enterprise may want remote session persistence.

A benchmark runner may want one directory per trajectory.

These should all be storage extensions, not forks.

```rust
pub struct JsonlReplayStoreExtension;

impl RoderExtension for JsonlReplayStoreExtension {
    fn install(
        &self,
        registry: &mut ExtensionRegistryBuilder,
    ) -> anyhow::Result<()> {
        registry.session_store_factory(Arc::new(JsonlSessionStoreFactory));
        registry.checkpoint_store_factory(Arc::new(JsonlCheckpointStoreFactory));
        Ok(())
    }
}
```

Then config chooses:

```toml
[session]
store = "jsonl-replay"
path = "./runs"

[checkpoint]
snapshot_every = 50
```

The rest of the harness does not change.

---

# 26. The core invariant

Roder's core invariant is this:

> Everything important that happens in an agent run should be represented as a typed event, attached to a session, governed by capabilities, and reproducible through a stable runtime model.

This invariant supports both product and research use.

For product builders, it means reliability, user trust, and debuggability.

For labs, it means reproducibility, comparable experiments, and traceable trajectories.

For extension authors, it means a stable contract.

For the open-source project, it means changes can be made without breaking the ecosystem.

---

# 27. Long-term vision

If Roder succeeds, the ecosystem should change in a few visible ways.

New coding agents should not need to build their own harness from scratch. They should start with Roder and focus on product, UX, inference strategy, or domain-specific intelligence.

Research labs should be able to publish inference engines, context planners, memory systems, and evaluation environments as Roder extensions.

Model providers should be able to provide high-quality Roder integrations without requiring product teams to rewrite their runtime.

Training systems should be able to use Roder as a trajectory generator, replay engine, and environment substrate.

Open-source tools should be able to share a common runtime vocabulary for sessions, turns, tools, events, and traces.

A project like a user-facing coding assistant should be able to build on Roder rather than beside it.

Roder becomes the shared harness layer.

---

# 28. Closing statement

The AI coding-agent ecosystem is still early, but the shape of the infrastructure is becoming clear. Agents need more than model calls. They need runtimes. They need controlled access to tools, files, processes, context, memory, storage, policy, and user interaction. They need to be inspectable, replayable, extensible, and portable.

Today, too many teams are rebuilding that runtime from scratch.

Roder exists to provide the foundation once.

It is Rust-native because the foundation should be solid. It is extensible because the ecosystem is moving too quickly for a closed core. It is provider-neutral because no single model API should define the architecture. It is open source because the best ideas should be able to enter the core. It is forkable because freedom matters, but designed so that serious users can extend instead of fork.

Roder is not the final coding agent.

Roder is the harness beneath them.

The goal is ambitious: a maintained, reference-quality, extensible agent harness that labs, builders, researchers, and products can rely on.

The last harness ever written is not one that predicts every future use case. It is one whose architecture can absorb them.

That is Roder's mission.
