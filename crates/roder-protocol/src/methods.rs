use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerMethodSpec {
    pub method: &'static str,
    pub params_type: String,
    pub result_type: String,
    pub stability: AppServerMethodStability,
    pub feature_group: &'static str,
    pub idempotency: AppServerIdempotency,
    pub side_effect: AppServerSideEffect,
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub notifications: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppServerMethodStability {
    Stable,
    Experimental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppServerIdempotency {
    Idempotent,
    NonIdempotent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppServerSideEffect {
    ReadOnly,
    LocalState,
    ExternalProcess,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerMethodManifest {
    pub schema_version: u32,
    pub unknown_methods_allowed: bool,
    pub methods: Vec<AppServerMethodSpec>,
}

pub const APP_SERVER_METHOD_MANIFEST_VERSION: u32 = 1;

pub fn app_server_method_manifest() -> AppServerMethodManifest {
    AppServerMethodManifest {
        schema_version: APP_SERVER_METHOD_MANIFEST_VERSION,
        unknown_methods_allowed: true,
        methods: app_server_method_specs(),
    }
}

pub fn app_server_method_specs() -> Vec<AppServerMethodSpec> {
    METHOD_SPECS
        .iter()
        .map(|seed| AppServerMethodSpec {
            method: seed.method,
            params_type: method_type_name(seed.method, "Params"),
            result_type: method_type_name(seed.method, "Result"),
            stability: seed.stability,
            feature_group: seed.feature_group,
            idempotency: seed.idempotency,
            side_effect: seed.side_effect,
            notifications: seed.notifications,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppServerMethodSpecSeed {
    method: &'static str,
    stability: AppServerMethodStability,
    feature_group: &'static str,
    idempotency: AppServerIdempotency,
    side_effect: AppServerSideEffect,
    notifications: &'static [&'static str],
}

fn method_type_name(method: &str, suffix: &str) -> String {
    if let Some(name) = explicit_method_type_name(method, suffix) {
        return name.to_string();
    }
    let mut name = String::new();
    let mut capitalize_next = true;
    for ch in method.chars() {
        if ch.is_ascii_alphanumeric() {
            if capitalize_next {
                name.push(ch.to_ascii_uppercase());
                capitalize_next = false;
            } else {
                name.push(ch);
            }
        } else {
            capitalize_next = true;
        }
    }
    name.push_str(suffix);
    name
}

fn explicit_method_type_name(method: &str, suffix: &str) -> Option<&'static str> {
    match (method, suffix) {
        ("vcs/status", "Params") | ("vcs/lines/list", "Params") => Some("VcsWorkspaceParams"),
        ("vcs/status", "Result") => Some("VcsStatus"),
        ("vcs/changes/read", "Result") => Some("VcsChangedContentPage"),
        ("vcs/lines/list", "Result") => Some("Vec<VcsLineOfWork>"),
        ("vcs/lines/switch", "Params") => Some("VcsLineSwitchParams"),
        ("vcs/lines/switch", "Result")
        | ("vcs/restore", "Result")
        | ("vcs/select", "Result")
        | ("vcs/sync", "Result") => Some("VcsOperationResult"),
        ("vcs/select", "Params") => Some("VcsSelectionParams"),
        ("vcs/snapshot/create", "Result") => Some("VcsSnapshot"),
        _ => None,
    }
}

macro_rules! method_spec {
    ($method:literal, $group:literal, $effect:ident, $idem:ident) => {
        AppServerMethodSpecSeed {
            method: $method,
            stability: AppServerMethodStability::Stable,
            feature_group: $group,
            idempotency: AppServerIdempotency::$idem,
            side_effect: AppServerSideEffect::$effect,
            notifications: &[],
        }
    };
    ($method:literal, $group:literal, $effect:ident, $idem:ident, [$($notification:literal),+ $(,)?]) => {
        AppServerMethodSpecSeed {
            method: $method,
            stability: AppServerMethodStability::Stable,
            feature_group: $group,
            idempotency: AppServerIdempotency::$idem,
            side_effect: AppServerSideEffect::$effect,
            notifications: &[$($notification),+],
        }
    };
}

const METHOD_SPECS: &[AppServerMethodSpecSeed] = &[
    method_spec!("agents/list", "agents", ReadOnly, Idempotent),
    method_spec!("artifact/delete", "artifacts", LocalState, NonIdempotent),
    method_spec!("artifact/grep", "artifacts", ReadOnly, Idempotent),
    method_spec!("artifact/list", "artifacts", ReadOnly, Idempotent),
    method_spec!("artifact/read", "artifacts", ReadOnly, Idempotent),
    method_spec!("artifact/tail", "artifacts", ReadOnly, Idempotent),
    method_spec!("auth/codex/login", "auth", ExternalProcess, NonIdempotent),
    method_spec!("auth/codex/logout", "auth", LocalState, NonIdempotent),
    method_spec!("auth/codex/status", "auth", ReadOnly, Idempotent),
    method_spec!(
        "auth/kimi-code/login",
        "auth",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("auth/kimi-code/logout", "auth", LocalState, NonIdempotent),
    method_spec!("auth/kimi-code/status", "auth", ReadOnly, Idempotent),
    method_spec!(
        "auth/supergrok/login",
        "auth",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("auth/supergrok/logout", "auth", LocalState, NonIdempotent),
    method_spec!("auth/supergrok/status", "auth", ReadOnly, Idempotent),
    method_spec!(
        "automations/cancelRun",
        "automations",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "automations/create",
        "automations",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "automations/delete",
        "automations",
        LocalState,
        NonIdempotent
    ),
    method_spec!("automations/list", "automations", ReadOnly, Idempotent),
    method_spec!(
        "automations/runNow",
        "automations",
        LocalState,
        NonIdempotent
    ),
    method_spec!("automations/runs", "automations", ReadOnly, Idempotent),
    method_spec!("automations/status", "automations", ReadOnly, Idempotent),
    method_spec!(
        "automations/update",
        "automations",
        LocalState,
        NonIdempotent
    ),
    method_spec!("chrome/browsers/list", "chrome", ReadOnly, Idempotent),
    method_spec!("chrome/debug/console", "chrome", ReadOnly, Idempotent),
    method_spec!("chrome/debug/network", "chrome", ReadOnly, Idempotent),
    method_spec!("chrome/disable", "chrome", LocalState, NonIdempotent),
    method_spec!("chrome/enable", "chrome", LocalState, NonIdempotent),
    method_spec!(
        "chrome/page/action",
        "chrome",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("chrome/page/snapshot", "chrome", ReadOnly, Idempotent),
    method_spec!("chrome/permissions/list", "chrome", ReadOnly, Idempotent),
    method_spec!(
        "chrome/permissions/update",
        "chrome",
        LocalState,
        NonIdempotent
    ),
    method_spec!("chrome/reconnect", "chrome", ReadOnly, Idempotent),
    method_spec!("chrome/setMode", "chrome", LocalState, NonIdempotent),
    method_spec!("chrome/status", "chrome", ReadOnly, Idempotent),
    method_spec!(
        "chrome/tabs/activate",
        "chrome",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("chrome/tabs/list", "chrome", ReadOnly, Idempotent),
    method_spec!(
        "chrome/tabs/navigate",
        "chrome",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("command/exec", "commands", ExternalProcess, NonIdempotent),
    method_spec!("commands/expand", "commands", ReadOnly, Idempotent),
    method_spec!("commands/list", "commands", ReadOnly, Idempotent),
    method_spec!("commands/run", "commands", LocalState, NonIdempotent),
    method_spec!("design/batch_get", "design", ReadOnly, Idempotent),
    method_spec!(
        "design/export_nodes",
        "design",
        LocalState,
        NonIdempotent,
        ["design/exportCompleted"]
    ),
    method_spec!("design/get_editor_state", "design", ReadOnly, Idempotent),
    method_spec!("design/get_guidelines", "design", ReadOnly, Idempotent),
    method_spec!("design/get_screenshot", "design", ReadOnly, Idempotent),
    method_spec!("design/get_variables", "design", ReadOnly, Idempotent),
    method_spec!(
        "design/patch",
        "design",
        LocalState,
        NonIdempotent,
        ["design/documentChanged"]
    ),
    method_spec!("design/read", "design", ReadOnly, Idempotent),
    method_spec!(
        "design/set_selection",
        "design",
        LocalState,
        NonIdempotent,
        ["design/selectionChanged"]
    ),
    method_spec!(
        "design/set_variables",
        "design",
        LocalState,
        NonIdempotent,
        ["design/documentChanged"]
    ),
    method_spec!("design/snapshot_layout", "design", ReadOnly, Idempotent),
    method_spec!("design/spawn_agents", "design", ReadOnly, NonIdempotent),
    method_spec!("discovery/groups", "discovery", ReadOnly, Idempotent),
    method_spec!("discovery/promote", "discovery", LocalState, NonIdempotent),
    method_spec!(
        "discovery/promoted/clear",
        "discovery",
        LocalState,
        NonIdempotent
    ),
    method_spec!("discovery/promoted/list", "discovery", ReadOnly, Idempotent),
    method_spec!("discovery/read", "discovery", ReadOnly, Idempotent),
    method_spec!("discovery/refresh", "discovery", LocalState, NonIdempotent),
    method_spec!("discovery/search", "discovery", ReadOnly, Idempotent),
    method_spec!("eval/report/read", "eval", ReadOnly, Idempotent),
    method_spec!("eval/reports/list", "eval", ReadOnly, Idempotent),
    method_spec!("extensions/list", "extensions", ReadOnly, Idempotent),
    method_spec!("forks/create", "forks", LocalState, NonIdempotent),
    method_spec!("forks/list", "forks", ReadOnly, Idempotent),
    method_spec!("forks/providers/list", "forks", ReadOnly, Idempotent),
    method_spec!("forks/remove", "forks", LocalState, NonIdempotent),
    method_spec!("fs/readDirectory", "filesystem", ReadOnly, Idempotent),
    method_spec!("fs/readFile", "filesystem", ReadOnly, Idempotent),
    method_spec!("hosted/audit/list", "hosted", ReadOnly, Idempotent),
    method_spec!("hosted/hooks/create", "hosted", LocalState, NonIdempotent),
    method_spec!("hosted/hooks/delete", "hosted", LocalState, NonIdempotent),
    method_spec!("hosted/hooks/list", "hosted", ReadOnly, Idempotent),
    method_spec!("hosted/hooks/update", "hosted", LocalState, NonIdempotent),
    method_spec!(
        "hosted/service_accounts/create",
        "hosted",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "hosted/service_accounts/list",
        "hosted",
        ReadOnly,
        Idempotent
    ),
    method_spec!(
        "hosted/service_accounts/revoke",
        "hosted",
        LocalState,
        NonIdempotent
    ),
    method_spec!("hosted/tenant/read", "hosted", ReadOnly, Idempotent),
    method_spec!("hosted/tenants/list", "hosted", ReadOnly, Idempotent),
    method_spec!("hosted/usage/read", "hosted", ReadOnly, Idempotent),
    method_spec!("hosted/whoami", "hosted", ReadOnly, Idempotent),
    method_spec!("hunk/list", "plan-review", ReadOnly, Idempotent),
    method_spec!("hunk/read", "plan-review", ReadOnly, Idempotent),
    method_spec!("hunk/rollback", "plan-review", LocalState, NonIdempotent),
    method_spec!("index/proofs/list", "code-index", ReadOnly, Idempotent),
    method_spec!("index/readChunk", "code-index", ReadOnly, Idempotent),
    method_spec!("index/rebuild", "code-index", LocalState, NonIdempotent),
    method_spec!("index/search", "code-index", ReadOnly, Idempotent),
    method_spec!("index/status", "code-index", ReadOnly, Idempotent),
    method_spec!(
        "inference/routing/metrics",
        "inference",
        ReadOnly,
        Idempotent
    ),
    method_spec!(
        "inference/routing/status",
        "inference",
        ReadOnly,
        Idempotent
    ),
    method_spec!("initialize", "app", ReadOnly, Idempotent),
    method_spec!("knowledge/delete", "knowledge", LocalState, NonIdempotent),
    method_spec!(
        "knowledge/links/set",
        "knowledge",
        LocalState,
        NonIdempotent
    ),
    method_spec!("knowledge/list", "knowledge", ReadOnly, Idempotent),
    method_spec!("knowledge/read", "knowledge", ReadOnly, Idempotent),
    method_spec!(
        "knowledge/revisions/list",
        "knowledge",
        ReadOnly,
        Idempotent
    ),
    method_spec!("knowledge/save", "knowledge", LocalState, NonIdempotent),
    method_spec!("knowledge/search", "knowledge", ReadOnly, Idempotent),
    method_spec!("knowledge/update", "knowledge", LocalState, NonIdempotent),
    method_spec!("lifecycle/metrics", "lifecycle", ReadOnly, Idempotent),
    method_spec!(
        "marketplaces/add",
        "marketplaces",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "marketplaces/install_default",
        "marketplaces",
        LocalState,
        NonIdempotent
    ),
    method_spec!("marketplaces/list", "marketplaces", ReadOnly, Idempotent),
    method_spec!("marketplaces/plugin", "marketplaces", ReadOnly, Idempotent),
    method_spec!(
        "marketplaces/refresh",
        "marketplaces",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "marketplaces/remove",
        "marketplaces",
        LocalState,
        NonIdempotent
    ),
    method_spec!("marketplaces/search", "marketplaces", ReadOnly, Idempotent),
    method_spec!("media/attachToTurn", "media", LocalState, NonIdempotent),
    method_spec!("media/delete", "media", LocalState, NonIdempotent),
    method_spec!("media/image/generate", "media", LocalState, NonIdempotent),
    method_spec!("media/image/providers/list", "media", ReadOnly, Idempotent),
    method_spec!("media/list", "media", ReadOnly, Idempotent),
    method_spec!("media/read", "media", ReadOnly, Idempotent),
    method_spec!("media/thumbnail", "media", ReadOnly, Idempotent),
    method_spec!("memory/delete", "memory", LocalState, NonIdempotent),
    method_spec!("memory/list", "memory", ReadOnly, Idempotent),
    method_spec!("memory/provider/list", "memory", ReadOnly, Idempotent),
    method_spec!("memory/provider/set", "memory", LocalState, NonIdempotent),
    method_spec!("memory/query", "memory", ReadOnly, Idempotent),
    method_spec!("memory/read", "memory", ReadOnly, Idempotent),
    method_spec!("memory/recall/preview", "memory", ReadOnly, Idempotent),
    method_spec!("memory/save", "memory", LocalState, NonIdempotent),
    method_spec!("memory/update", "memory", LocalState, NonIdempotent),
    method_spec!("model/list", "models", ReadOnly, Idempotent),
    method_spec!("model/select", "models", LocalState, NonIdempotent),
    method_spec!("node/status", "node", ReadOnly, Idempotent),
    method_spec!(
        "packages/approve_extensions",
        "packages",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "packages/install",
        "packages",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("packages/list", "packages", ReadOnly, Idempotent),
    method_spec!("packages/remove", "packages", LocalState, NonIdempotent),
    method_spec!(
        "packages/set_enabled",
        "packages",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "packages/set_filters",
        "packages",
        LocalState,
        NonIdempotent
    ),
    method_spec!("packages/sync", "packages", ExternalProcess, NonIdempotent),
    method_spec!(
        "packages/update",
        "packages",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!(
        "plan/review/approve",
        "plan-review",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "plan/review/comment",
        "plan-review",
        LocalState,
        NonIdempotent
    ),
    method_spec!("plan/review/read", "plan-review", ReadOnly, Idempotent),
    method_spec!(
        "plan/review/reject",
        "plan-review",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "plan/review/rewrite",
        "plan-review",
        LocalState,
        NonIdempotent
    ),
    method_spec!("plugins/disable", "plugins", LocalState, NonIdempotent),
    method_spec!("plugins/install", "plugins", LocalState, NonIdempotent),
    method_spec!(
        "plugins/install_all_variants",
        "plugins",
        LocalState,
        NonIdempotent
    ),
    method_spec!("plugins/list_installed", "plugins", ReadOnly, Idempotent),
    method_spec!("plugins/preview_install", "plugins", ReadOnly, Idempotent),
    method_spec!("plugins/uninstall", "plugins", LocalState, NonIdempotent),
    method_spec!("processes/get", "processes", ReadOnly, Idempotent),
    method_spec!("processes/list", "processes", ReadOnly, Idempotent),
    method_spec!("processes/stop", "processes", LocalState, NonIdempotent),
    method_spec!("processes/stopAll", "processes", LocalState, NonIdempotent),
    method_spec!(
        "processes/subscribe",
        "processes",
        ReadOnly,
        Idempotent,
        ["processes/changed"]
    ),
    method_spec!("providers/clear", "providers", LocalState, NonIdempotent),
    method_spec!(
        "providers/configure",
        "providers",
        LocalState,
        NonIdempotent
    ),
    method_spec!("providers/list", "providers", ReadOnly, Idempotent),
    method_spec!("providers/select", "providers", LocalState, NonIdempotent),
    method_spec!("retrieval/metrics", "retrieval", ReadOnly, Idempotent),
    method_spec!("retrieval/promoted", "retrieval", ReadOnly, Idempotent),
    method_spec!(
        "retrieval/recommendations",
        "retrieval",
        ReadOnly,
        Idempotent
    ),
    method_spec!("roadmap/create", "roadmap", LocalState, NonIdempotent),
    method_spec!("roadmap/list", "roadmap", ReadOnly, Idempotent),
    method_spec!("roadmap/patch", "roadmap", LocalState, NonIdempotent),
    method_spec!("roadmap/read", "roadmap", ReadOnly, Idempotent),
    method_spec!("roadmap/task/update", "roadmap", LocalState, NonIdempotent),
    method_spec!(
        "roadmap/thread/attach",
        "roadmap",
        LocalState,
        NonIdempotent
    ),
    method_spec!("roadmap/thread/list", "roadmap", ReadOnly, Idempotent),
    method_spec!(
        "roadmap/thread/spawn",
        "roadmap",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("roadmap/validate", "roadmap", ReadOnly, Idempotent),
    method_spec!("runners/delete", "runners", LocalState, NonIdempotent),
    method_spec!("runners/detach", "runners", ExternalProcess, NonIdempotent),
    method_spec!("runners/list", "runners", ReadOnly, Idempotent),
    method_spec!("runners/pause", "runners", ExternalProcess, NonIdempotent),
    method_spec!("runners/ports", "runners", ReadOnly, Idempotent),
    method_spec!("runners/rejoin", "runners", ExternalProcess, NonIdempotent),
    method_spec!("runners/resume", "runners", ExternalProcess, NonIdempotent),
    method_spec!("runners/select", "runners", LocalState, NonIdempotent),
    method_spec!("runners/session", "runners", ReadOnly, Idempotent),
    method_spec!("runners/snapshot", "runners", ReadOnly, Idempotent),
    method_spec!(
        "runtime/drain",
        "runtime",
        LocalState,
        NonIdempotent,
        ["turn/lifecycleUpdated", "turn/completed"]
    ),
    method_spec!(
        "search_index/clear",
        "search-index",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "search_index/rebuild",
        "search-index",
        LocalState,
        NonIdempotent
    ),
    method_spec!("search_index/status", "search-index", ReadOnly, Idempotent),
    method_spec!(
        "search_index/warmup",
        "search-index",
        LocalState,
        NonIdempotent
    ),
    method_spec!("settings/get", "settings", ReadOnly, Idempotent),
    method_spec!(
        "settings/set_default_mode",
        "settings",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "settings/set_file_backed_dynamic_context",
        "settings",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "settings/set_search_index",
        "settings",
        LocalState,
        NonIdempotent
    ),
    method_spec!("settings/set_shell", "settings", LocalState, NonIdempotent),
    method_spec!(
        "settings/set_web_search",
        "settings",
        LocalState,
        NonIdempotent
    ),
    method_spec!("skills/list", "skills", ReadOnly, Idempotent),
    method_spec!("skills/read", "skills", ReadOnly, Idempotent),
    method_spec!("skills/setEnabled", "skills", LocalState, NonIdempotent),
    method_spec!("skills/setExposure", "skills", LocalState, NonIdempotent),
    method_spec!("speech/providers/list", "speech", ReadOnly, Idempotent),
    method_spec!(
        "speech/synthesis/providers/list",
        "speech",
        ReadOnly,
        Idempotent
    ),
    method_spec!(
        "speech/synthesize",
        "speech",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!(
        "speech/transcribe",
        "speech",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("stats/backfill", "stats", LocalState, NonIdempotent),
    method_spec!("stats/export", "stats", LocalState, NonIdempotent),
    method_spec!("stats/sessions", "stats", ReadOnly, Idempotent),
    method_spec!("stats/summary", "stats", ReadOnly, Idempotent),
    method_spec!("stats/tokens", "stats", ReadOnly, Idempotent),
    method_spec!("stats/tools", "stats", ReadOnly, Idempotent),
    method_spec!("tasks/cancel", "tasks", LocalState, NonIdempotent),
    method_spec!("tasks/get", "tasks", ReadOnly, Idempotent),
    method_spec!("tasks/list", "tasks", ReadOnly, Idempotent),
    method_spec!("tasks/submit", "tasks", LocalState, NonIdempotent),
    method_spec!(
        "tasks/subscribe",
        "tasks",
        ReadOnly,
        Idempotent,
        [
            "task.started",
            "task.output",
            "task.completed",
            "task.failed",
            "task.cancelled",
        ]
    ),
    method_spec!("team/cleanup", "teams", LocalState, NonIdempotent),
    method_spec!("team/list", "teams", ReadOnly, Idempotent),
    method_spec!("team/member/focus", "teams", LocalState, NonIdempotent),
    method_spec!("team/member/interrupt", "teams", LocalState, NonIdempotent),
    method_spec!("team/member/message", "teams", LocalState, NonIdempotent),
    method_spec!("team/member/start", "teams", LocalState, NonIdempotent),
    method_spec!("team/pane/cleanup", "teams", LocalState, NonIdempotent),
    method_spec!("team/pane/focus", "teams", LocalState, NonIdempotent),
    method_spec!("team/read", "teams", ReadOnly, Idempotent),
    method_spec!("team/start", "teams", LocalState, NonIdempotent),
    method_spec!("thread/archive", "thread", LocalState, NonIdempotent),
    method_spec!("thread/attach", "thread", LocalState, NonIdempotent),
    method_spec!(
        "thread/compact",
        "thread",
        LocalState,
        NonIdempotent,
        ["context.compaction_started", "context.compaction_recorded"]
    ),
    method_spec!("thread/exit_plan", "thread", LocalState, NonIdempotent),
    method_spec!("thread/fork", "thread", LocalState, NonIdempotent),
    method_spec!("thread/fork_status", "thread", ReadOnly, Idempotent),
    method_spec!(
        "thread/goal/clear",
        "thread",
        LocalState,
        NonIdempotent,
        ["thread/goal/cleared"]
    ),
    method_spec!("thread/goal/get", "thread", ReadOnly, Idempotent),
    method_spec!(
        "thread/goal/set",
        "thread",
        LocalState,
        NonIdempotent,
        ["thread/goal/updated"]
    ),
    method_spec!("thread/list", "thread", ReadOnly, Idempotent),
    method_spec!("thread/read", "thread", ReadOnly, Idempotent),
    method_spec!("thread/remove_fork", "thread", LocalState, NonIdempotent),
    method_spec!(
        "thread/resolve_approval",
        "thread",
        LocalState,
        NonIdempotent
    ),
    method_spec!(
        "thread/resolve_user_input",
        "thread",
        LocalState,
        NonIdempotent
    ),
    method_spec!("thread/roadmap/open", "thread", LocalState, NonIdempotent),
    method_spec!(
        "thread/set_agent_swarm_mode",
        "thread",
        LocalState,
        NonIdempotent
    ),
    method_spec!("thread/set_mode", "thread", LocalState, NonIdempotent),
    method_spec!("thread/start", "thread", LocalState, NonIdempotent),
    method_spec!("thread/state", "thread", ReadOnly, Idempotent),
    method_spec!("tools/call", "tools", LocalState, NonIdempotent),
    method_spec!("tools/list", "tools", ReadOnly, Idempotent),
    method_spec!("tools/resolve", "tools", LocalState, NonIdempotent),
    method_spec!("turn/interrupt", "turns", LocalState, NonIdempotent),
    method_spec!("turn/start", "turns", LocalState, NonIdempotent),
    method_spec!("turn/steer", "turns", LocalState, NonIdempotent),
    method_spec!("turn/subagentTrace/read", "turns", ReadOnly, Idempotent),
    method_spec!("turn/subagentTraces/list", "turns", ReadOnly, Idempotent),
    method_spec!("vcs/changes/list", "vcs", ReadOnly, Idempotent),
    method_spec!("vcs/changes/read", "vcs", ReadOnly, Idempotent),
    method_spec!("vcs/lines/list", "vcs", ReadOnly, Idempotent),
    method_spec!("vcs/lines/switch", "vcs", LocalState, NonIdempotent),
    method_spec!("vcs/restore", "vcs", LocalState, NonIdempotent),
    method_spec!("vcs/select", "vcs", LocalState, NonIdempotent),
    method_spec!("vcs/snapshot/create", "vcs", LocalState, NonIdempotent),
    method_spec!("vcs/status", "vcs", ReadOnly, Idempotent),
    method_spec!("vcs/sync", "vcs", ExternalProcess, NonIdempotent),
    method_spec!("webwright/artifacts", "webwright", ReadOnly, Idempotent),
    method_spec!("webwright/export", "webwright", LocalState, NonIdempotent),
    method_spec!("webwright/latestRun", "webwright", ReadOnly, Idempotent),
    method_spec!("webwright/prepare", "webwright", LocalState, NonIdempotent),
    method_spec!("webwright/report", "webwright", ReadOnly, Idempotent),
    method_spec!(
        "webwright/rerun",
        "webwright",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!(
        "webwright/setup",
        "webwright",
        ExternalProcess,
        NonIdempotent
    ),
    method_spec!("webwright/submit", "webwright", LocalState, NonIdempotent),
    method_spec!("webwright/verify", "webwright", ReadOnly, Idempotent),
    method_spec!(
        "webwright/visualJudge",
        "webwright",
        LocalState,
        NonIdempotent
    ),
    method_spec!("workflow/enable", "workflow", LocalState, NonIdempotent),
    method_spec!("workflow/ignore", "workflow", LocalState, NonIdempotent),
    method_spec!("workflow/preview", "workflow", ReadOnly, Idempotent),
    method_spec!("workflow/refresh", "workflow", LocalState, NonIdempotent),
    method_spec!("workflow/remove", "workflow", LocalState, NonIdempotent),
    method_spec!("workflow/scan", "workflow", ReadOnly, Idempotent),
    method_spec!(
        "workflows/approve",
        "workflows",
        LocalState,
        NonIdempotent,
        [
            "workflows/approved",
            "workflows/denied",
            "workflows/queued",
            "workflows/started",
        ]
    ),
    method_spec!("workflows/get", "workflows", ReadOnly, Idempotent),
    method_spec!("workflows/list", "workflows", ReadOnly, Idempotent),
    method_spec!(
        "workflows/pause",
        "workflows",
        LocalState,
        NonIdempotent,
        ["workflows/paused"]
    ),
    method_spec!(
        "workflows/plan",
        "workflows",
        LocalState,
        NonIdempotent,
        ["workflows/drafted", "workflows/approvalRequested"]
    ),
    method_spec!(
        "workflows/restartAgent",
        "workflows",
        LocalState,
        NonIdempotent,
        [
            "workflows/agentQueued",
            "workflows/agentStarted",
            "workflows/agentCompleted",
            "workflows/agentFailed",
        ]
    ),
    method_spec!(
        "workflows/resume",
        "workflows",
        LocalState,
        NonIdempotent,
        ["workflows/resumed"]
    ),
    method_spec!("workflows/save", "workflows", LocalState, NonIdempotent),
    method_spec!(
        "workflows/scripts/delete",
        "workflows",
        LocalState,
        NonIdempotent
    ),
    method_spec!("workflows/scripts/list", "workflows", ReadOnly, Idempotent),
    method_spec!("workflows/scripts/read", "workflows", ReadOnly, Idempotent),
    method_spec!(
        "workflows/stop",
        "workflows",
        LocalState,
        NonIdempotent,
        ["workflows/stopped"]
    ),
    method_spec!(
        "workspace/changes/list",
        "workspace",
        ReadOnly,
        Idempotent,
        ["workspace/changeObserved"]
    ),
    method_spec!("workspace/create", "workspace", LocalState, NonIdempotent),
    method_spec!(
        "workspace/files/children",
        "workspace-files",
        ReadOnly,
        Idempotent
    ),
    method_spec!(
        "workspace/files/query",
        "workspace-files",
        ReadOnly,
        Idempotent
    ),
    method_spec!(
        "workspace/files/read",
        "workspace-files",
        ReadOnly,
        Idempotent
    ),
    method_spec!(
        "workspace/files/rebuild",
        "workspace-files",
        LocalState,
        NonIdempotent,
        ["workspace/files/statusChanged"]
    ),
    method_spec!(
        "workspace/files/status",
        "workspace-files",
        ReadOnly,
        Idempotent
    ),
    method_spec!("workspace/forget", "workspace", LocalState, NonIdempotent),
    method_spec!("workspace/list", "workspace", ReadOnly, Idempotent),
    method_spec!("workspace/update", "workspace", LocalState, NonIdempotent),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn method_manifest_is_sorted_and_unique() {
        let methods = app_server_method_specs();
        let names = methods.iter().map(|spec| spec.method).collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);

        let unique = names.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn method_manifest_covers_broad_app_server_surface() {
        let methods = app_server_method_specs()
            .iter()
            .map(|spec| spec.method)
            .collect::<BTreeSet<_>>();
        for required in [
            "providers/list",
            "settings/get",
            "thread/start",
            "turn/start",
            "thread/resolve_approval",
            "tools/call",
            "commands/list",
            "tasks/submit",
            "webwright/prepare",
            "team/member/message",
            "plan/review/comment",
            "hunk/rollback",
            "workflow/scan",
            "workflows/plan",
            "marketplaces/search",
            "plugins/install",
            "packages/install",
            "packages/list",
            "media/list",
            "memory/query",
            "automations/list",
            "processes/list",
            "speech/providers/list",
            "speech/synthesis/providers/list",
            "speech/synthesize",
            "speech/transcribe",
            "vcs/status",
            "vcs/select",
            "workspace/files/children",
            "workspace/files/query",
            "workspace/files/read",
            "workspace/files/rebuild",
            "workspace/files/status",
        ] {
            assert!(methods.contains(required), "missing {required}");
        }
        assert!(!methods.contains("vcs/extras/list"));
    }

    #[test]
    fn method_manifest_uses_canonical_vcs_type_names() {
        let manifest = app_server_method_manifest();
        let status = manifest
            .methods
            .iter()
            .find(|method| method.method == "vcs/status")
            .expect("vcs/status method");
        assert_eq!(status.params_type, "VcsWorkspaceParams");
        assert_eq!(status.result_type, "VcsStatus");

        let select = manifest
            .methods
            .iter()
            .find(|method| method.method == "vcs/select")
            .expect("vcs/select method");
        assert_eq!(select.params_type, "VcsSelectionParams");
        assert_eq!(select.result_type, "VcsOperationResult");
    }

    #[test]
    fn method_manifest_records_subscription_notifications() {
        let methods = app_server_method_specs();
        let processes = methods
            .iter()
            .find(|spec| spec.method == "processes/subscribe")
            .expect("processes subscribe spec");
        assert_eq!(processes.notifications, ["processes/changed"]);

        let tasks = methods
            .iter()
            .find(|spec| spec.method == "tasks/subscribe")
            .expect("tasks subscribe spec");
        assert!(tasks.notifications.contains(&"task.started"));
        assert!(tasks.notifications.contains(&"task.completed"));

        let workflows_plan = methods
            .iter()
            .find(|spec| spec.method == "workflows/plan")
            .expect("workflows plan spec");
        assert!(workflows_plan.notifications.contains(&"workflows/drafted"));
        assert!(
            workflows_plan
                .notifications
                .contains(&"workflows/approvalRequested")
        );

        let workspace_files_rebuild = methods
            .iter()
            .find(|spec| spec.method == "workspace/files/rebuild")
            .expect("workspace files rebuild spec");
        assert_eq!(
            workspace_files_rebuild.notifications,
            ["workspace/files/statusChanged"]
        );
    }
}
