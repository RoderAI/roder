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
        self._source_roots = tuple(
            kwargs.get("source_roots", ("Cargo.toml", "Cargo.lock", ".cargo", "crates"))
        )
        soft_timeout = kwargs.get("soft_timeout_sec") or os.environ.get(
            "RODER_HARBOR_SOFT_TIMEOUT_SEC"
        )
        self._soft_timeout_sec = int(float(soft_timeout)) if soft_timeout else None

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
        setup = (
            f"mkdir -p {config_dir}/auth /logs/agent && "
            f"touch {shlex.quote(events_path)} {shlex.quote(stderr_path)} "
            f"{shlex.quote(output_path)} {shlex.quote(last_message_path)} "
            f"{shlex.quote(setup_summary_path)} && "
            f"printf 'Roder run command setup started\\n' >> {shlex.quote(setup_summary_path)} && "
            "if [ -f /installed-agent/roder-auth.json ]; then "
            f"cp /installed-agent/roder-auth.json {config_dir}/auth/codex.json; "
            "fi && "
            f"cat > {config_dir}/config.toml <<'EOF'\n"
            f"provider = {json.dumps(provider)}\n"
            f"model = {json.dumps(model)}\n"
            f"reasoning = {json.dumps(str(self._reasoning))}\n"
            "runtime_profile = \"eval\"\n"
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
            "soft_timed_out=0\n"
            f"printf 'roder exec starting\\n' >> {shlex.quote(setup_summary_path)}\n"
            + self._roder_exec_shell_fragment(
                events_path=events_path,
                stderr_path=stderr_path,
                last_message_path=last_message_path,
            )
            + "status=$?\n"
            "case \"$status\" in 124|130|137|143) soft_timed_out=1 ;; esac\n"
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
            "if [ \"$soft_timed_out\" -eq 1 ]; then "
            f"printf 'roder exec soft-timed-out before Harbor hard timeout\\n' >> {shlex.quote(setup_summary_path)}; "
            "exit 0; "
            "fi\n"
            "exit \"$status\"\n"
        )
        run = f"bash -lc {shlex.quote(run_script)}"
        env = {
            "RODER_CONFIG_DIR": config_dir,
            "RODER_DATA_DIR": config_dir,
            "RODER_HARBOR_PROMPT": instruction,
        }
        return [
            ExecInput(command=setup, env=env),
            ExecInput(command=run, env=env),
        ]

    def _roder_exec_shell_fragment(
        self, events_path: str, stderr_path: str, last_message_path: str
    ) -> str:
        command = (
            f"roder exec --json --profile eval --mode {shlex.quote(str(self._policy_mode))} "
            f"--skip-git-repo-check --output-last-message {shlex.quote(last_message_path)} - "
            f">{shlex.quote(events_path)} 2>{shlex.quote(stderr_path)}"
        )
        if not self._soft_timeout_sec:
            return f"printf '%s' \"$RODER_HARBOR_PROMPT\" | {command}\n"
        timeout = shlex.quote(f"{self._soft_timeout_sec}s")
        return (
            "if command -v timeout >/dev/null 2>&1; then\n"
            f"  printf '%s' \"$RODER_HARBOR_PROMPT\" | timeout -k 30s -s INT {timeout} {command}\n"
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
