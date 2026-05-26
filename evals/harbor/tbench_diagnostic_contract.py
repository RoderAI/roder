"""Shared contract for local Terminal-Bench diagnostic fixtures."""

EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES = (
    "tbench-exact-output-file",
    "tbench-json-array-output",
    "tbench-numeric-tolerance-output",
    "tbench-output-directory-hygiene",
    "tbench-sequence-output",
    "tbench-visible-verifier-contract",
    "tbench-artifact-checkpoint",
    "tbench-service-target-sanity",
    "tbench-verifier-dependency-parity",
)

EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS = {
    "tbench-numeric-tolerance-output": 1,
    "tbench-output-directory-hygiene": 1,
    "tbench-visible-verifier-contract": 1,
    "tbench-artifact-checkpoint": 1,
    "tbench-service-target-sanity": 1,
    "tbench-verifier-dependency-parity": 1,
}

EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS = {
    "tbench-artifact-checkpoint": {
        "updates": 2,
        "completed": 2,
    },
}
