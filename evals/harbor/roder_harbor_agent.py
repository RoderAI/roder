import os
import shlex
import tarfile
import tempfile
import time
from pathlib import Path

try:
    from harbor.agents.installed.base import BaseInstalledAgent, ExecInput
except ImportError:
    from dataclasses import dataclass

    from harbor.agents.installed.base import BaseInstalledAgent

    @dataclass
    class ExecInput:
        command: str
        env: dict[str, str] | None = None
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.trial.paths import EnvironmentPaths

from roder_benchmark_guidance import TERMINAL_BENCH_GUIDANCE
from roder_config_shell import rewrite_reasoning_shell_fragment
from roder_harbor_agent_config import (
    optional_bool,
    optional_float,
    optional_int,
    optional_int_list,
    reliability_config_toml,
    speed_policy_config_toml,
)
from roder_harbor_agent_settings import parse_agent_settings
from roder_harbor_run_script import build_run_agent_commands
from roder_plan_first import (
    implementation_prompt_for_instruction,
    plan_prompt_for_instruction,
)
from roder_policy_block_retry import (
    policy_block_check_command,
    policy_block_retry_budget_sec,
)
from roder_signal_recovery import signal_recovery_shell_fragment
from tbench_deadline_policy import derive_task_deadline_ladder
from tbench_task_windows import (
    lookup_task_agent_timeout_sec,
    task_name_from_logs_dir,
)


PROVIDER_ENV_KEYS = {
    "gemini": (
        "GEMINI_API_TOKEN",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GOOGLE_GENAI_API_KEY",
        "GOOGLE_AI_API_KEY",
    ),
}

SIGNAL_TERMINATED_STATUSES = {124, 130, 137, 143}


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
        settings = parse_agent_settings(
            kwargs, source_dir_default=Path(__file__).resolve().parents[2]
        )
        self._settings = settings
        self._provider = settings.provider
        self._reasoning = settings.reasoning
        self._policy_mode = settings.policy_mode
        self._source_dir = settings.source_dir
        self._auth_file = settings.auth_file
        self._include_local_source = settings.include_local_source
        self._include_prebuilt_binary = settings.include_prebuilt_binary
        self._prebuilt_binary = settings.prebuilt_binary
        self._prebuilt_binary_amd64 = settings.prebuilt_binary_amd64
        self._prebuilt_binary_arm64 = settings.prebuilt_binary_arm64
        self._benchmark_guidance_enabled = settings.benchmark_guidance_enabled
        self._task_ledger_required = settings.task_ledger_required
        self._plan_first_enabled = settings.plan_first_enabled
        self._plan_first_policy_mode = settings.plan_first_policy_mode
        self._plan_first_reasoning = settings.plan_first_reasoning
        self._plan_first_soft_timeout_sec = settings.plan_first_soft_timeout_sec
        self._source_roots = settings.source_roots
        self._soft_timeout_sec = settings.soft_timeout_sec
        self._per_task_deadlines = settings.per_task_deadlines
        self._agent_timeout_multiplier_hint = settings.agent_timeout_multiplier_hint
        self._task_cache_dir = settings.task_cache_dir
        self._policy_block_max_retries = settings.policy_block_max_retries
        self._speed_policy_enabled = settings.speed_policy_enabled
        self._speed_policy_eval_deadline_seconds = (
            settings.speed_policy_eval_deadline_seconds
        )
        self._speed_policy_reasoning = settings.speed_policy_reasoning
        self._reliability = settings.reliability
        self._tool_allowlist = settings.tool_allowlist

    @property
    def _install_agent_template_path(self) -> Path:
        return Path(__file__).with_name("install-roder.sh.j2")

    def _setup_env(self) -> dict[str, str]:
        setup_env = getattr(super(), "_setup_env", None)
        env = setup_env() if setup_env else {}
        for key in ("RODER_HARBOR_GIT_URL", "RODER_HARBOR_GIT_REF"):
            if value := os.environ.get(key):
                env[key] = value
        return env

    async def install(self, environment: BaseEnvironment) -> None:
        command = self._install_agent_template_path.read_text()
        result = await environment.exec(
            command=f"bash -lc {shlex.quote(command)}",
            user="root",
            env=self._setup_env(),
            timeout_sec=2400,
        )
        if result.return_code != 0:
            raise RuntimeError(f"Roder install failed with status {result.return_code}")

    def _resolved_provider_model(self) -> tuple[str, str]:
        model_name = self.model_name or "codex/gpt-5.5"
        if "/" in model_name:
            provider, model = model_name.split("/", 1)
        else:
            provider = self._provider or "codex"
            model = model_name
        return self._provider or provider, model

    def _provider_env(self, provider: str) -> dict[str, str]:
        return {
            key: value
            for key in PROVIDER_ENV_KEYS.get(provider, ())
            if (value := os.environ.get(key))
        }

    def _resolved_task_name(self) -> str | None:
        return task_name_from_logs_dir(getattr(self, "logs_dir", None))

    def _resolved_deadlines(self) -> tuple[int | None, int | None]:
        """Return (soft_timeout_sec, eval_deadline_seconds) for this trial.

        When ``per_task_deadlines`` is on and the task's declared Terminal-Bench
        window is resolvable, the soft timeout and roder turn deadline are
        derived from that window so the agent uses the benchmark's own time
        budget instead of a single shortened global deadline. Otherwise the
        statically configured values are used unchanged.
        """
        if not self._per_task_deadlines:
            return self._soft_timeout_sec, self._speed_policy_eval_deadline_seconds
        task_name = self._resolved_task_name()
        task_timeout = lookup_task_agent_timeout_sec(
            task_name, cache_dir=self._task_cache_dir
        )
        ladder = derive_task_deadline_ladder(
            task_timeout,
            agent_timeout_multiplier=self._agent_timeout_multiplier_hint,
        )
        if ladder is None:
            return self._soft_timeout_sec, self._speed_policy_eval_deadline_seconds
        return ladder.soft_timeout_sec, ladder.eval_deadline_seconds

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
        uploaded_prebuilt = False
        if self._include_prebuilt_binary and self._prebuilt_binary_amd64.exists():
            await environment.upload_file(
                source_path=self._prebuilt_binary_amd64,
                target_path="/installed-agent/roder-linux-amd64",
            )
            uploaded_prebuilt = True
        if self._include_prebuilt_binary and self._prebuilt_binary_arm64.exists():
            await environment.upload_file(
                source_path=self._prebuilt_binary_arm64,
                target_path="/installed-agent/roder-linux-arm64",
            )
            uploaded_prebuilt = True
        if self._include_prebuilt_binary and self._prebuilt_binary.exists():
            await environment.upload_file(
                source_path=self._prebuilt_binary,
                target_path="/installed-agent/roder",
            )
            uploaded_prebuilt = True
        if not uploaded_prebuilt and self._include_local_source:
            archive_path = self._create_source_archive()
            await environment.upload_file(
                source_path=archive_path,
                target_path="/installed-agent/roder-source.tar.gz",
            )
        elif not uploaded_prebuilt:
            raise FileNotFoundError(
                "No prebuilt Linux roder binary found. Run "
                "./evals/harbor/build-prebuilt-roder.sh or set "
                "RODER_HARBOR_PREBUILT_BINARY_AMD64/ARM64."
            )
        if self._auth_file.exists():
            await environment.upload_file(
                source_path=self._auth_file,
                target_path="/installed-agent/roder-auth.json",
            )
        await super().setup(environment)

    def create_run_agent_commands(self, instruction: str) -> list[ExecInput]:
        return [
            ExecInput(command=command, env=env)
            for command, env in build_run_agent_commands(self, instruction)
        ]

    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        commands = self.create_run_agent_commands(instruction)
        run_command = commands[-1]
        for exec_input in commands[:-1]:
            result = await environment.exec(
                command=exec_input.command,
                env=exec_input.env,
            )
            if result.return_code != 0:
                raise RuntimeError(
                    f"Roder command failed with status {result.return_code}"
                )

        run_started = time.monotonic()
        result = await environment.exec(
            command=run_command.command,
            env=run_command.env,
        )
        if result.return_code != 0:
            if (
                result.return_code in SIGNAL_TERMINATED_STATUSES
                and await self._recover_signal_terminated_run(
                    environment=environment,
                    signal_status=result.return_code,
                )
            ):
                return
            raise RuntimeError(
                f"Roder command failed with status {result.return_code}"
            )

        await self._maybe_retry_zero_progress_policy_block(
            environment=environment,
            run_command=run_command,
            run_started=run_started,
        )

    async def _maybe_retry_zero_progress_policy_block(
        self,
        *,
        environment: BaseEnvironment,
        run_command: ExecInput,
        run_started: float,
    ) -> None:
        """Re-run on a fresh thread when the provider policy-blocked with no tool use.

        Only a block that occurred before any tool call is retried: the agent
        did no task work, the block is stochastic (the same prompt usually
        passes on a new thread), and the retry stays inside the unmodified task
        window. Retries are bounded and skipped when insufficient soft-timeout
        budget remains.
        """
        if self._policy_block_max_retries <= 0:
            return
        soft_timeout_sec, _ = self._resolved_deadlines()
        agent_dir = EnvironmentPaths.agent_dir
        check_command = policy_block_check_command(
            events_path=(agent_dir / "roder-events.jsonl").as_posix(),
            stderr_path=(agent_dir / "roder-stderr.txt").as_posix(),
        )
        for _ in range(self._policy_block_max_retries):
            check = await environment.exec(command=check_command)
            if check.return_code != 0:
                return
            budget = policy_block_retry_budget_sec(
                soft_timeout_sec=soft_timeout_sec,
                elapsed_sec=time.monotonic() - run_started,
            )
            if budget is None:
                return
            retry = await environment.exec(
                command=run_command.command,
                env=run_command.env,
            )
            if retry.return_code != 0:
                if (
                    retry.return_code in SIGNAL_TERMINATED_STATUSES
                    and await self._recover_signal_terminated_run(
                        environment=environment,
                        signal_status=retry.return_code,
                    )
                ):
                    return
                raise RuntimeError(
                    f"Roder command failed with status {retry.return_code}"
                )

    async def _recover_signal_terminated_run(
        self,
        *,
        environment: BaseEnvironment,
        signal_status: int,
    ) -> bool:
        provider, model = self._resolved_provider_model()
        config_dir = "/tmp/roder-harbor"
        agent_dir = EnvironmentPaths.agent_dir
        command = signal_recovery_shell_fragment(
            signal_status=signal_status,
            provider=provider,
            model=model,
            reasoning=str(self._reasoning),
            policy_mode=str(self._policy_mode),
            task_ledger_required=self._task_ledger_required,
            soft_timeout_sec=self._soft_timeout_sec,
            eval_deadline_seconds=self._speed_policy_eval_deadline_seconds,
            config_dir=config_dir,
            events_path=(agent_dir / "roder-events.jsonl").as_posix(),
            stderr_path=(agent_dir / "roder-stderr.txt").as_posix(),
            output_path=(agent_dir / "roder-cli.txt").as_posix(),
            last_message_path=(agent_dir / "roder-last-message.txt").as_posix(),
            setup_summary_path=(agent_dir / "setup-summary.txt").as_posix(),
            run_summary_path=(agent_dir / "roder-run-summary.json").as_posix(),
        )
        result = await environment.exec(
            command=f"bash -lc {shlex.quote(command)}",
            env={
                "RODER_CONFIG_DIR": config_dir,
                "RODER_DATA_DIR": config_dir,
            },
            timeout_sec=120,
        )
        return result.return_code == 0

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
