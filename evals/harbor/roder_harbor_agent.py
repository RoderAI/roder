import json
import os
import shlex
import tarfile
import tempfile
from pathlib import Path

from harbor.agents.installed.base import BaseInstalledAgent, ExecInput
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.trial.paths import EnvironmentPaths

from roder_benchmark_guidance import TERMINAL_BENCH_GUIDANCE
from roder_config_shell import rewrite_reasoning_shell_fragment
from roder_exec_shell import roder_exec_shell_fragment
from roder_harbor_agent_config import (
    optional_bool,
    optional_float,
    optional_int,
    optional_int_list,
    reliability_config_toml,
    speed_policy_config_toml,
)
from roder_plan_first import (
    implementation_prompt_for_instruction,
    plan_first_shell_fragment,
    plan_prompt_for_instruction,
)
from roder_run_summary_fragment import run_summary_shell_fragment


class RoderCli(BaseInstalledAgent):
    @staticmethod
    def name() -> str:
        return "roder-cli"

    def __init__(self, model_name: str | None = None, *args, **kwargs):
        super().__init__(
            model_name=model_name or kwargs.get("default_model") or "codex/gpt-5.5",
            *args,
            **kwargs,
        )
        self._provider = kwargs.get("provider")
        self._reasoning = kwargs.get("reasoning", "medium")
        self._policy_mode = kwargs.get("policy_mode", "bypass")
        self._source_dir = Path(
            kwargs.get("source_dir")
            or os.environ.get("RODER_HARBOR_SOURCE_DIR")
            or Path(__file__).resolve().parents[2]
        ).expanduser()
        self._auth_file = Path(
            kwargs.get("auth_file")
            or os.environ.get("RODER_HARBOR_AUTH_FILE")
            or "~/.roder/auth/codex.json"
        ).expanduser()
        self._include_local_source = str(
            kwargs.get("include_local_source", "true")
        ).lower() not in {"0", "false", "no"}
        self._include_prebuilt_binary = str(
            kwargs.get("include_prebuilt_binary", "true")
        ).lower() not in {"0", "false", "no"}
        self._prebuilt_binary = Path(
            kwargs.get("prebuilt_binary")
            or os.environ.get("RODER_HARBOR_PREBUILT_BINARY")
            or self._source_dir / "evals/harbor/artifacts/roder-linux-amd64"
        ).expanduser()
        guidance = optional_bool(
            kwargs.get("benchmark_guidance_enabled")
            if "benchmark_guidance_enabled" in kwargs
            else os.environ.get("RODER_HARBOR_BENCHMARK_GUIDANCE_ENABLED")
        )
        self._benchmark_guidance_enabled = True if guidance is None else guidance
        task_ledger = optional_bool(
            kwargs.get("task_ledger_required")
            if "task_ledger_required" in kwargs
            else os.environ.get("RODER_HARBOR_TASK_LEDGER_REQUIRED")
        )
        self._task_ledger_required = False if task_ledger is None else task_ledger
        plan_first = optional_bool(
            kwargs.get("plan_first_enabled")
            if "plan_first_enabled" in kwargs
            else os.environ.get("RODER_HARBOR_PLAN_FIRST_ENABLED")
        )
        self._plan_first_enabled = False if plan_first is None else plan_first
        self._plan_first_policy_mode = str(
            kwargs.get("plan_first_policy_mode")
            or os.environ.get("RODER_HARBOR_PLAN_FIRST_POLICY_MODE")
            or self._policy_mode
        )
        self._plan_first_reasoning = str(
            kwargs.get("plan_first_reasoning")
            or os.environ.get("RODER_HARBOR_PLAN_FIRST_REASONING")
            or self._reasoning
        )
        plan_first_soft_timeout = kwargs.get("plan_first_soft_timeout_sec") or os.environ.get(
            "RODER_HARBOR_PLAN_FIRST_SOFT_TIMEOUT_SEC"
        )
        self._plan_first_soft_timeout_sec = optional_int(plan_first_soft_timeout)
        if self._plan_first_enabled and self._plan_first_soft_timeout_sec is None:
            self._plan_first_soft_timeout_sec = 360
        self._source_roots = tuple(
            kwargs.get("source_roots", ("Cargo.toml", "Cargo.lock", ".cargo", "crates"))
        )
        soft_timeout = kwargs.get("soft_timeout_sec") or os.environ.get(
            "RODER_HARBOR_SOFT_TIMEOUT_SEC"
        )
        self._soft_timeout_sec = int(float(soft_timeout)) if soft_timeout else None
        self._speed_policy_enabled = optional_bool(
            kwargs.get("speed_policy_enabled")
            if "speed_policy_enabled" in kwargs
            else os.environ.get("RODER_HARBOR_SPEED_POLICY_ENABLED")
        )
        self._speed_policy_eval_deadline_seconds = optional_int(
            kwargs.get("speed_policy_eval_deadline_seconds")
            if "speed_policy_eval_deadline_seconds" in kwargs
            else os.environ.get("RODER_HARBOR_SPEED_POLICY_EVAL_DEADLINE_SECONDS")
        )
        self._speed_policy_reasoning = {
            "orientation_reasoning": kwargs.get("speed_policy_orientation_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_ORIENTATION_REASONING"),
            "execution_reasoning": kwargs.get("speed_policy_execution_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_EXECUTION_REASONING"),
            "verification_reasoning": kwargs.get("speed_policy_verification_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_VERIFICATION_REASONING"),
            "recovery_reasoning": kwargs.get("speed_policy_recovery_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_RECOVERY_REASONING"),
        }
        self._reliability = {
            "provider_retry_max_attempts": optional_int(
                kwargs.get("reliability_provider_retry_max_attempts")
                if "reliability_provider_retry_max_attempts" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_MAX_ATTEMPTS")
            ),
            "provider_retry_initial_backoff_ms": optional_int(
                kwargs.get("reliability_provider_retry_initial_backoff_ms")
                if "reliability_provider_retry_initial_backoff_ms" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_INITIAL_BACKOFF_MS")
            ),
            "provider_retry_backoff_factor": optional_float(
                kwargs.get("reliability_provider_retry_backoff_factor")
                if "reliability_provider_retry_backoff_factor" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_BACKOFF_FACTOR")
            ),
            "provider_retry_status_codes": optional_int_list(
                kwargs.get("reliability_provider_retry_status_codes")
                if "reliability_provider_retry_status_codes" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_STATUS_CODES")
            ),
            "retry_empty_provider_body": optional_bool(
                kwargs.get("reliability_retry_empty_provider_body")
                if "reliability_retry_empty_provider_body" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_RETRY_EMPTY_PROVIDER_BODY")
            ),
            "max_consecutive_tool_failures": optional_int(
                kwargs.get("reliability_max_consecutive_tool_failures")
                if "reliability_max_consecutive_tool_failures" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_MAX_CONSECUTIVE_TOOL_FAILURES")
            ),
            "max_tool_failures_per_turn": optional_int(
                kwargs.get("reliability_max_tool_failures_per_turn")
                if "reliability_max_tool_failures_per_turn" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_MAX_TOOL_FAILURES_PER_TURN")
            ),
            "max_model_calls_per_turn": optional_int(
                kwargs.get("reliability_max_model_calls_per_turn")
                if "reliability_max_model_calls_per_turn" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_MAX_MODEL_CALLS_PER_TURN")
            ),
        }

    @property
    def _install_agent_template_path(self) -> Path:
        return Path(__file__).with_name("install-roder.sh.j2")

    def _setup_env(self) -> dict[str, str]:
        env = super()._setup_env()
        for key in ("RODER_HARBOR_GIT_URL", "RODER_HARBOR_GIT_REF"):
            if value := os.environ.get(key):
                env[key] = value
        return env

    def _resolved_provider_model(self) -> tuple[str, str]:
        model_name = self.model_name or "codex/gpt-5.5"
        if "/" in model_name:
            provider, model = model_name.split("/", 1)
        else:
            provider = self._provider or "codex"
            model = model_name
        return self._provider or provider, model

    def _prompt_for_instruction(self, instruction: str) -> str:
        if not self._benchmark_guidance_enabled:
            return instruction
        return f"{TERMINAL_BENCH_GUIDANCE}\n\n{instruction}"

    def _plan_first_env(self, instruction: str) -> dict[str, str]:
        if not self._plan_first_enabled:
            return {"RODER_HARBOR_PROMPT": self._prompt_for_instruction(instruction)}
        terminal_guidance = (
            TERMINAL_BENCH_GUIDANCE if self._benchmark_guidance_enabled else ""
        )
        return {
            "RODER_HARBOR_PLAN_PROMPT": plan_prompt_for_instruction(instruction),
            "RODER_HARBOR_PROMPT": implementation_prompt_for_instruction(
                terminal_bench_guidance=terminal_guidance,
                instruction=instruction,
            ),
        }

    def _create_source_archive(self) -> Path:
        if not self._source_dir.exists():
            raise FileNotFoundError(f"Roder source dir not found: {self._source_dir}")
        temp_dir = Path(tempfile.mkdtemp(prefix="roder-harbor-source-"))
        archive_path = temp_dir / "roder-source.tar.gz"
        excludes = {
            ".git",
            ".roder",
            "target",
            ".target-roadmap-local",
            "evals/harbor/artifacts",
            "evals/harbor/jobs",
            "evals/reports",
            ".DS_Store",
        }

        def excluded(path: Path) -> bool:
            rel = path.relative_to(self._source_dir).as_posix()
            return any(rel == item or rel.startswith(f"{item}/") for item in excludes)

        with tarfile.open(archive_path, "w:gz") as archive:
            for root in self._source_roots:
                root_path = self._source_dir / root
                if not root_path.exists() or excluded(root_path):
                    continue
                if root_path.is_file():
                    archive.add(root_path, arcname=root_path.relative_to(self._source_dir))
                    continue
                for dirpath, dirnames, filenames in os.walk(root_path):
                    current_dir = Path(dirpath)
                    dirnames[:] = [
                        dirname
                        for dirname in dirnames
                        if not excluded(current_dir / dirname)
                    ]
                    for filename in filenames:
                        path = current_dir / filename
                        if not excluded(path):
                            archive.add(path, arcname=path.relative_to(self._source_dir))
        return archive_path

    async def setup(self, environment: BaseEnvironment) -> None:
        await environment.exec(command="mkdir -p /installed-agent")
        if self._include_prebuilt_binary and self._prebuilt_binary.exists():
            await environment.upload_file(
                source_path=self._prebuilt_binary,
                target_path="/installed-agent/roder",
            )
        elif self._include_local_source:
            archive_path = self._create_source_archive()
            await environment.upload_file(
                source_path=archive_path,
                target_path="/installed-agent/roder-source.tar.gz",
            )
        else:
            raise FileNotFoundError(
                "No prebuilt Linux roder binary found. Run "
                "./evals/harbor/build-prebuilt-roder.sh or set "
                "RODER_HARBOR_PREBUILT_BINARY."
            )
        if self._auth_file.exists():
            await environment.upload_file(
                source_path=self._auth_file,
                target_path="/installed-agent/roder-auth.json",
            )
        await super().setup(environment)

    def create_run_agent_commands(self, instruction: str) -> list[ExecInput]:
        provider, model = self._resolved_provider_model()
        config_dir = "/tmp/roder-harbor"
        events_path = (EnvironmentPaths.agent_dir / "roder-events.jsonl").as_posix()
        stderr_path = (EnvironmentPaths.agent_dir / "roder-stderr.txt").as_posix()
        output_path = (EnvironmentPaths.agent_dir / "roder-cli.txt").as_posix()
        last_message_path = (EnvironmentPaths.agent_dir / "roder-last-message.txt").as_posix()
        setup_summary_path = (EnvironmentPaths.agent_dir / "setup-summary.txt").as_posix()
        run_summary_path = (EnvironmentPaths.agent_dir / "roder-run-summary.json").as_posix()
        plan_events_path = (EnvironmentPaths.agent_dir / "roder-plan-events.jsonl").as_posix()
        plan_stderr_path = (EnvironmentPaths.agent_dir / "roder-plan-stderr.txt").as_posix()
        plan_last_message_path = (
            EnvironmentPaths.agent_dir / "roder-plan-last-message.txt"
        ).as_posix()
        plan_path = (EnvironmentPaths.agent_dir / "roder-plan.md").as_posix()
        setup = (
            f"mkdir -p {config_dir}/auth /logs/agent && "
            f"touch {shlex.quote(events_path)} {shlex.quote(stderr_path)} "
            f"{shlex.quote(output_path)} {shlex.quote(last_message_path)} "
            f"{shlex.quote(setup_summary_path)} {shlex.quote(run_summary_path)} "
            f"{shlex.quote(plan_events_path)} {shlex.quote(plan_stderr_path)} "
            f"{shlex.quote(plan_last_message_path)} {shlex.quote(plan_path)} && "
            f"printf 'Roder run command setup started\\n' >> {shlex.quote(setup_summary_path)} && "
            "if [ -f /installed-agent/roder-auth.json ]; then "
            f"cp /installed-agent/roder-auth.json {config_dir}/auth/codex.json; "
            "fi && "
            f"cat > {config_dir}/config.toml <<'EOF'\n"
            f"provider = {json.dumps(provider)}\n"
            f"model = {json.dumps(model)}\n"
            f"reasoning = {json.dumps(str(self._reasoning))}\n"
            "runtime_profile = \"eval\"\n"
            f"{speed_policy_config_toml(enabled=self._speed_policy_enabled, eval_deadline_seconds=self._speed_policy_eval_deadline_seconds, reasoning=self._speed_policy_reasoning)}"
            f"{reliability_config_toml(self._reliability)}"
            "\n"
            "[policy_modes]\n"
            f"default = {json.dumps(str(self._policy_mode))}\n"
            "warn_on_bypass = false\n"
            "EOF"
        )
        run_script = (
            "set -uo pipefail\n"
            f": > {shlex.quote(events_path)}\n"
            f": > {shlex.quote(stderr_path)}\n"
            f": > {shlex.quote(output_path)}\n"
            f": > {shlex.quote(last_message_path)}\n"
            f": > {shlex.quote(run_summary_path)}\n"
            "start_epoch=$(date +%s)\n"
            "started_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')\n"
            "soft_timed_out=0\n"
            "deadline_timed_out=0\n"
            "RODER_HARBOR_PLAN_THREAD_ID=\n"
            f"printf 'roder exec starting\\n' >> {shlex.quote(setup_summary_path)}\n"
            + self._reasoning_shell_fragment(
                config_dir=config_dir,
                setup_summary_path=setup_summary_path,
                reasoning=self._plan_first_reasoning,
                label="plan-first planning",
            )
            + plan_first_shell_fragment(
                enabled=self._plan_first_enabled,
                events_path=plan_events_path,
                stderr_path=plan_stderr_path,
                last_message_path=plan_last_message_path,
                plan_path=plan_path,
                setup_summary_path=setup_summary_path,
                policy_mode=self._plan_first_policy_mode,
                soft_timeout_sec=self._plan_first_soft_timeout_sec,
                task_ledger_required=self._task_ledger_required,
            )
            + self._reasoning_shell_fragment(
                config_dir=config_dir,
                setup_summary_path=setup_summary_path,
                reasoning=str(self._reasoning),
                label="implementation",
            )
            + roder_exec_shell_fragment(
                events_path=events_path,
                stderr_path=stderr_path,
                last_message_path=last_message_path,
                prompt_env_var="RODER_HARBOR_PROMPT",
                policy_mode=str(self._policy_mode),
                soft_timeout_sec=self._soft_timeout_sec,
                task_ledger_required=self._task_ledger_required,
                resume_thread_env_var=(
                    "RODER_HARBOR_PLAN_THREAD_ID"
                    if self._plan_first_enabled
                    else None
                ),
            )
            + "status=$?\n"
            "case \"$status\" in 124|130|137|143) soft_timed_out=1 ;; esac\n"
            f"if grep -q 'turn deadline expired' {shlex.quote(stderr_path)}; then "
            "deadline_timed_out=1; soft_timed_out=1; "
            "fi\n"
            f"if [ -s {shlex.quote(last_message_path)} ]; then "
            f"cp {shlex.quote(last_message_path)} {shlex.quote(output_path)}; "
            "else "
            f"printf 'roder exec exited with status %s before writing a final message\\n' \"$status\" > {shlex.quote(output_path)}; "
            "fi\n"
            f"if [ -s {shlex.quote(stderr_path)} ]; then "
            f"printf '\\n--- roder stderr ---\\n' >> {shlex.quote(output_path)}; "
            f"cat {shlex.quote(stderr_path)} >> {shlex.quote(output_path)}; "
            "fi\n"
            f"printf 'roder exec finished with status %s\\n' \"$status\" >> {shlex.quote(setup_summary_path)}\n"
            "if [ \"$deadline_timed_out\" -eq 1 ]; then "
            f"printf 'roder exec hit internal eval deadline before Harbor hard timeout\\n' >> {shlex.quote(setup_summary_path)}; "
            "fi\n"
            + run_summary_shell_fragment(
                provider=provider,
                model=model,
                reasoning=str(self._reasoning),
                policy_mode=str(self._policy_mode),
                task_ledger_required=self._task_ledger_required,
                soft_timeout_sec=self._soft_timeout_sec,
                eval_deadline_seconds=self._speed_policy_eval_deadline_seconds,
                config_dir=config_dir,
                events_path=events_path,
                stderr_path=stderr_path,
                output_path=output_path,
                last_message_path=last_message_path,
                run_summary_path=run_summary_path,
            )
            + "if [ \"$soft_timed_out\" -eq 1 ]; then "
            f"printf 'roder exec soft-timed-out before Harbor hard timeout\\n' >> {shlex.quote(setup_summary_path)}; "
            "exit 0; "
            "fi\n"
            "exit \"$status\"\n"
        )
        run = f"bash -lc {shlex.quote(run_script)}"
        env = {
            "RODER_CONFIG_DIR": config_dir,
            "RODER_DATA_DIR": config_dir,
        }
        env.update(self._plan_first_env(instruction))
        return [
            ExecInput(command=setup, env=env),
            ExecInput(command=run, env=env),
        ]

    def _reasoning_shell_fragment(
        self,
        *,
        config_dir: str,
        setup_summary_path: str,
        reasoning: str,
        label: str,
    ) -> str:
        if not self._plan_first_enabled:
            return ""
        return rewrite_reasoning_shell_fragment(
            config_dir=config_dir,
            reasoning=reasoning,
            setup_summary_path=setup_summary_path,
            label=label,
        )

    def populate_context_post_run(self, context: AgentContext) -> None:
        output_path = self.logs_dir / "roder-cli.txt"
        if output_path.exists():
            metadata = dict(context.metadata or {})
            metadata["roder_output_path"] = str(output_path)
            context.metadata = metadata
