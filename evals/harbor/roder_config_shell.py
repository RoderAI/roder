"""Shell fragments for mutating the generated Harbor Roder config."""

from __future__ import annotations

import shlex


def rewrite_reasoning_shell_fragment(
    *, config_dir: str, reasoning: str, setup_summary_path: str, label: str
) -> str:
    config_path = f"{config_dir}/config.toml"
    return (
        f"python3 - {shlex.quote(config_path)} {shlex.quote(reasoning)} <<'PY'\n"
        "import json\n"
        "import sys\n"
        "from pathlib import Path\n"
        "\n"
        "path = Path(sys.argv[1])\n"
        "reasoning = sys.argv[2]\n"
        "lines = path.read_text().splitlines()\n"
        "for index, line in enumerate(lines):\n"
        "    if line.startswith('reasoning = '):\n"
        "        lines[index] = 'reasoning = ' + json.dumps(reasoning)\n"
        "        break\n"
        "else:\n"
        "    lines.append('reasoning = ' + json.dumps(reasoning))\n"
        "path.write_text('\\n'.join(lines) + '\\n')\n"
        "PY\n"
        f"printf 'roder config reasoning for {label}: {shlex.quote(reasoning)}\\n' "
        f">> {shlex.quote(setup_summary_path)}\n"
    )
