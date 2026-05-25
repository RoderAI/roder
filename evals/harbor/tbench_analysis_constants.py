"""Shared constants for Harbor Terminal-Bench analysis."""

HARNESS_ERROR_CLASSES = {
    "docker_registry_bad_gateway",
    "agent_setup_failed",
    "agent_timeout",
    "missing_artifacts",
    "verifier_error",
    "unknown_error",
}

CORE_ARTIFACTS = (
    "roder-cli.txt",
    "roder-events.jsonl",
    "roder-stderr.txt",
    "roder-last-message.txt",
)

RUN_SUMMARY_TASK_FIELDS = (
    "elapsed_seconds",
    "soft_timed_out",
    "deadline_timed_out",
    "deadline_finalized",
    "provider_error_kind",
    "stderr_noise_kind",
    "active_tool",
    "last_tool",
)

SCORED_GROUP_PATTERNS = {
    "ML/scientific": (
        "torch",
        "train-fasttext",
        "mteb",
        "raman",
        "mcmc",
        "protein",
        "financial-document",
        "tune-mjcf",
        "count-dataset",
        "query-optimize",
    ),
    "systems/emulation/services": (
        "kv-store",
        "mailman",
        "install-windows",
        "mips",
        "make-doom",
        "polyglot-rust-c",
    ),
    "media/geometry": (
        "path-tracing",
        "video-processing",
        "gcode",
    ),
    "synthesis/security/math": (
        "gpt2-codegolf",
        "regex",
        "overfull-hbox",
        "fix-code-vulnerability",
        "chess",
        "winning-avg-corewars",
    ),
}

SCORED_GROUP_SUBSYSTEMS = {
    "ML/scientific": "runtime context, package-install planning, long-running command monitoring, and verification discipline",
    "systems/emulation/services": "shell/process tooling, service startup validation, and timeout/deadline handling",
    "media/geometry": "artifact inspection, binary/media tooling, and iterative verifier feedback",
    "synthesis/security/math": "search/context retrieval, exact-output discipline, and test-driven repair loops",
    "other": "task-specific analysis after clean harness artifacts are available",
}
