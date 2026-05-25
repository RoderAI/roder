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

from roder_run_summary_fragment import run_summary_shell_fragment


TERMINAL_BENCH_GUIDANCE = """Terminal-Bench harness guidance:
- The verifier scores filesystem and process state, not the final chat message.
- If the task asks for a file or exact answer, write the best current answer to the requested path as soon as you have it; revise it later if needed.
- If you find any plausible exact-answer candidate, create or update the required output file before doing additional research, ranking, browsing, or long-running validation.
- Before web/search or long research, inspect local task assets first: list the current directory, /app, and /tests when they exist, then grep visible tests and task files for expected paths, constants, answer formats, and fixture data.
- For exact-answer tasks, the final chat answer alone does not score. Create or update the required output file before finalizing, even if the answer is provisional.
- Required output files must contain complete machine-parseable content, not summaries. Never write ellipses, placeholders, Markdown fences, prose introductions, or truncated excerpts into files that the task will parse as sequences, JSON, CSV, SQL, code, model configs, or exact answers.
- After writing a required output file, validate the actual file on disk with the same parser or regex shape the task implies. Do not rely only on checks over an intermediate variable. For DNA or gBlock files, the final file should normally match `^[ACGTacgt]+$` exactly, with no ambiguous bases such as `N`, amino-acid letters, whitespace beyond the allowed newline, ellipses, or comments.
- For dated or "as of" leaderboard/data tasks, live web pages can be wrong and current live leaderboards are usually not the answer. Prefer local fixtures, package data, git history, release snapshots, or commits at or before the requested date, and record the dated answer in the required file before doing slower live-data checks.
- If current/live data conflicts with a dated or historical candidate, keep the dated or historical candidate in the required output file unless direct dated computation proves it wrong.
- For benchmark leaderboards, respect the requested benchmark/filter and score column exactly. Prefer complete or eligible benchmark rows over partial high-scoring rows when computing a Mean (Task) leaderboard unless the task explicitly says partial coverage counts.
- For MTEB embedding or retrieval tasks, prefer the installed `mteb` package APIs over raw `sentence_transformers` calls when the task mentions `mteb`: use `mteb.get_model`, the model's `encode`/`similarity` helpers, and `mteb.encoder_interface.PromptType.query` / `PromptType.passage` for retrieval-style cosine similarity instead of assuming one generic embedding path. Do not finalize an MTEB retrieval task until a local script has run the requested model/revision and written the computed rank to the required file.
- For document sorting/extraction tasks, process every input file before moving or deleting it. Keep a manifest of filenames, classify each file once, and verify the required summary has one row per required document plus any requested total row before finalizing.
- For database recovery, WAL, journal, or event-log tasks, replay the local log as ordered state changes. Apply inserts, updates, deletes, and replacements to the same in-memory records before writing the final export; validate changes to existing ids as well as new rows. For encrypted or corrupted binary logs, derive likely transforms from known file magic/header bytes, then query the repaired local database instead of fabricating monotonic placeholder values.
- For G-code, image, OCR, CAD, plot, or rendered-artifact tasks that ask for text or an exact string, inspect the source geometry and at least one rendered or transformed view. For 3D print paths, isolate extrusion moves above the base object, then try rotations, transposes, and mirrors before deciding; do not stop at the first readable generic word if a more exact or flag-like string may be visible from another projection.
- For sanitizer/filter tasks, preserve benign inputs according to the task's visible contract and local checks before finalizing. Build adversarial local checks for obfuscated dangerous forms, null bytes, malformed tags, unquoted attributes, JavaScript URLs, event handlers, CSS script patterns, and safe clean files. If using a parser or serializer such as BeautifulSoup, also compare clean examples against that parser's normalized output so attribute order, entity handling, void tags, and whitespace do not create false clean-file changes.
- Run quick local checks before long commands, and keep required output files scoreable before waiting on slow work.
- Preserve exact stdout, stderr, file names, file contents, dimensions, schemas, and exit behavior that the task describes or tests imply.
- Do not finish or mark the goal complete while a known local check is failing."""


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
        guidance = self._optional_bool(
            kwargs.get("benchmark_guidance_enabled")
            if "benchmark_guidance_enabled" in kwargs
            else os.environ.get("RODER_HARBOR_BENCHMARK_GUIDANCE_ENABLED")
        )
        self._benchmark_guidance_enabled = True if guidance is None else guidance
        task_ledger = self._optional_bool(
            kwargs.get("task_ledger_required")
            if "task_ledger_required" in kwargs
            else os.environ.get("RODER_HARBOR_TASK_LEDGER_REQUIRED")
        )
        self._task_ledger_required = False if task_ledger is None else task_ledger
        self._source_roots = tuple(
            kwargs.get("source_roots", ("Cargo.toml", "Cargo.lock", ".cargo", "crates"))
        )
        soft_timeout = kwargs.get("soft_timeout_sec") or os.environ.get(
            "RODER_HARBOR_SOFT_TIMEOUT_SEC"
        )
        self._soft_timeout_sec = int(float(soft_timeout)) if soft_timeout else None
        self._speed_policy_enabled = self._optional_bool(
            kwargs.get("speed_policy_enabled")
            if "speed_policy_enabled" in kwargs
            else os.environ.get("RODER_HARBOR_SPEED_POLICY_ENABLED")
        )
        self._speed_policy_eval_deadline_seconds = self._optional_int(
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
            "provider_retry_max_attempts": self._optional_int(
                kwargs.get("reliability_provider_retry_max_attempts")
                if "reliability_provider_retry_max_attempts" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_MAX_ATTEMPTS")
            ),
            "provider_retry_initial_backoff_ms": self._optional_int(
                kwargs.get("reliability_provider_retry_initial_backoff_ms")
                if "reliability_provider_retry_initial_backoff_ms" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_INITIAL_BACKOFF_MS")
            ),
            "provider_retry_backoff_factor": self._optional_int(
                kwargs.get("reliability_provider_retry_backoff_factor")
                if "reliability_provider_retry_backoff_factor" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_BACKOFF_FACTOR")
            ),
            "provider_retry_status_codes": self._optional_int_list(
                kwargs.get("reliability_provider_retry_status_codes")
                if "reliability_provider_retry_status_codes" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_PROVIDER_RETRY_STATUS_CODES")
            ),
            "retry_empty_provider_body": self._optional_bool(
                kwargs.get("reliability_retry_empty_provider_body")
                if "reliability_retry_empty_provider_body" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_RETRY_EMPTY_PROVIDER_BODY")
            ),
            "max_consecutive_tool_failures": self._optional_int(
                kwargs.get("reliability_max_consecutive_tool_failures")
                if "reliability_max_consecutive_tool_failures" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_MAX_CONSECUTIVE_TOOL_FAILURES")
            ),
            "max_tool_failures_per_turn": self._optional_int(
                kwargs.get("reliability_max_tool_failures_per_turn")
                if "reliability_max_tool_failures_per_turn" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_MAX_TOOL_FAILURES_PER_TURN")
            ),
            "max_model_calls_per_turn": self._optional_int(
                kwargs.get("reliability_max_model_calls_per_turn")
                if "reliability_max_model_calls_per_turn" in kwargs
                else os.environ.get("RODER_HARBOR_RELIABILITY_MAX_MODEL_CALLS_PER_TURN")
            ),
        }

    @staticmethod
    def _optional_bool(value) -> bool | None:
        if value is None:
            return None
        if isinstance(value, bool):
            return value
        text = str(value).strip().lower()
        if text in {"1", "true", "yes", "on"}:
            return True
        if text in {"0", "false", "no", "off"}:
            return False
        raise ValueError(f"invalid boolean value: {value!r}")

    @staticmethod
    def _optional_int(value) -> int | None:
        if value is None or value == "":
            return None
        return int(float(value))

    @staticmethod
    def _optional_int_list(value) -> list[int] | None:
        if value is None or value == "":
            return None
        if isinstance(value, (list, tuple)):
            return [int(item) for item in value]
        return [int(part.strip()) for part in str(value).split(",") if part.strip()]

    @staticmethod
    def _toml_value(value) -> str:
        if isinstance(value, bool):
            return str(value).lower()
        return json.dumps(value)

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

    def _speed_policy_config_toml(self) -> str:
        lines: list[str] = []
        if self._speed_policy_enabled is not None:
            lines.append(f"enabled = {str(self._speed_policy_enabled).lower()}")
        if self._speed_policy_eval_deadline_seconds is not None:
            lines.append(
                f"eval_deadline_seconds = {self._speed_policy_eval_deadline_seconds}"
            )
        for key, value in self._speed_policy_reasoning.items():
            if value:
                lines.append(f"{key} = {json.dumps(str(value))}")
        if not lines:
            return ""
        return "\n[speed_policy]\n" + "\n".join(lines) + "\n"

    def _reliability_config_toml(self) -> str:
        lines = [
            f"{key} = {self._toml_value(value)}"
            for key, value in self._reliability.items()
            if value is not None
        ]
        if not lines:
            return ""
        return "\n[reliability]\n" + "\n".join(lines) + "\n"

    def _prompt_for_instruction(self, instruction: str) -> str:
        if not self._benchmark_guidance_enabled:
            return instruction
        return f"{TERMINAL_BENCH_GUIDANCE}\n\n{instruction}"

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
        setup = (
            f"mkdir -p {config_dir}/auth /logs/agent && "
            f"touch {shlex.quote(events_path)} {shlex.quote(stderr_path)} "
            f"{shlex.quote(output_path)} {shlex.quote(last_message_path)} "
            f"{shlex.quote(setup_summary_path)} {shlex.quote(run_summary_path)} && "
            f"printf 'Roder run command setup started\\n' >> {shlex.quote(setup_summary_path)} && "
            "if [ -f /installed-agent/roder-auth.json ]; then "
            f"cp /installed-agent/roder-auth.json {config_dir}/auth/codex.json; "
            "fi && "
            f"cat > {config_dir}/config.toml <<'EOF'\n"
            f"provider = {json.dumps(provider)}\n"
            f"model = {json.dumps(model)}\n"
            f"reasoning = {json.dumps(str(self._reasoning))}\n"
            "runtime_profile = \"eval\"\n"
            f"{self._speed_policy_config_toml()}"
            f"{self._reliability_config_toml()}"
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
            f"printf 'roder exec starting\\n' >> {shlex.quote(setup_summary_path)}\n"
            + self._roder_exec_shell_fragment(
                events_path=events_path,
                stderr_path=stderr_path,
                last_message_path=last_message_path,
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
            "RODER_HARBOR_PROMPT": self._prompt_for_instruction(instruction),
        }
        return [
            ExecInput(command=setup, env=env),
            ExecInput(command=run, env=env),
        ]

    def _roder_exec_shell_fragment(
        self, events_path: str, stderr_path: str, last_message_path: str
    ) -> str:
        ledger_flag = " --task-ledger-required" if self._task_ledger_required else ""
        command = (
            f"roder exec --json --profile eval --mode {shlex.quote(str(self._policy_mode))} "
            f"--skip-git-repo-check{ledger_flag} "
            f"--output-last-message {shlex.quote(last_message_path)} - "
            f">{shlex.quote(events_path)} 2>{shlex.quote(stderr_path)}"
        )
        if not self._soft_timeout_sec:
            return f"printf '%s' \"$RODER_HARBOR_PROMPT\" | {command}\n"
        timeout = shlex.quote(f"{self._soft_timeout_sec}s")
        return (
            "if command -v timeout >/dev/null 2>&1; then\n"
            f"  printf '%s' \"$RODER_HARBOR_PROMPT\" | timeout -k 5s -s INT {timeout} {command}\n"
            "else\n"
            "  printf 'warning: timeout command unavailable; running without soft timeout\\n' "
            f">> {shlex.quote(stderr_path)}\n"
            f"  printf '%s' \"$RODER_HARBOR_PROMPT\" | {command}\n"
            "fi\n"
        )

    def populate_context_post_run(self, context: AgentContext) -> None:
        output_path = self.logs_dir / "roder-cli.txt"
        if output_path.exists():
            metadata = dict(context.metadata or {})
            metadata["roder_output_path"] = str(output_path)
            context.metadata = metadata
