#!/usr/bin/env bash
set -euo pipefail

DEFAULT_SESSION="${RODER_TMUX_SESSION:-roder}"
DEFAULT_OUT_DIR="${RODER_TMUX_OUT_DIR:-.roder/tmux-captures}"
DEFAULT_LINES="${RODER_TMUX_CAPTURE_LINES:-5000}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat <<'USAGE'
Usage:
  roder_tmux.sh start [-s session] [-d workdir] [-- command...]
  roder_tmux.sh send [-s session] [--enter] text
  roder_tmux.sh keys [-s session] key [key...]
  roder_tmux.sh capture [-s session] [-o out_dir] [-l lines]
  roder_tmux.sh attach [-s session]
  roder_tmux.sh stop [-s session]
  roder_tmux.sh status [-s session]

Defaults:
  session: roder
  command: cargo run -p roder-cli --bin roder
  output:  .roder/tmux-captures/
USAGE
}

die() {
  printf 'roder_tmux: %s\n' "$*" >&2
  exit 1
}

need_tmux() {
  command -v tmux >/dev/null 2>&1 || die "tmux is required"
}

target_for() {
  printf '%s:0.0' "$1"
}

session_exists() {
  tmux has-session -t "$1" 2>/dev/null
}

shell_join() {
  local out="" arg
  for arg in "$@"; do
    printf -v arg '%q' "$arg"
    out+="${out:+ }$arg"
  done
  printf '%s' "$out"
}

parse_session() {
  session="$DEFAULT_SESSION"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -s|--session)
        [[ $# -ge 2 ]] || die "missing value for $1"
        session="$2"
        shift 2
        ;;
      --)
        shift
        break
        ;;
      *)
        break
        ;;
    esac
  done
  remaining=("$@")
}

cmd_start() {
  need_tmux
  local session="$DEFAULT_SESSION"
  local workdir="$PWD"
  local -a command=()

  while [[ $# -gt 0 ]]; do
    case "$1" in
      -s|--session)
        [[ $# -ge 2 ]] || die "missing value for $1"
        session="$2"
        shift 2
        ;;
      -d|--dir|--workdir)
        [[ $# -ge 2 ]] || die "missing value for $1"
        workdir="$2"
        shift 2
        ;;
      --)
        shift
        command=("$@")
        break
        ;;
      *)
        command=("$@")
        break
        ;;
    esac
  done

  [[ -d "$workdir" ]] || die "workdir does not exist: $workdir"
  if session_exists "$session"; then
    printf 'session already exists: %s\n' "$session"
    return 0
  fi

  local shell_command
  if [[ ${#command[@]} -eq 0 ]]; then
    shell_command="cargo run -p roder-cli --bin roder"
  else
    shell_command="$(shell_join "${command[@]}")"
  fi

  tmux new-session -d -s "$session" -c "$workdir" "$shell_command"
  printf 'started %s with command: %s\n' "$session" "$shell_command"
}

cmd_send() {
  need_tmux
  local session="$DEFAULT_SESSION"
  local press_enter=0

  while [[ $# -gt 0 ]]; do
    case "$1" in
      -s|--session)
        [[ $# -ge 2 ]] || die "missing value for $1"
        session="$2"
        shift 2
        ;;
      --enter)
        press_enter=1
        shift
        ;;
      --no-enter)
        press_enter=0
        shift
        ;;
      --)
        shift
        break
        ;;
      *)
        break
        ;;
    esac
  done

  [[ $# -gt 0 ]] || die "send requires text"
  session_exists "$session" || die "session not found: $session"

  local text="$*"
  local buffer="roder-send-$$"
  printf '%s' "$text" | tmux load-buffer -b "$buffer" -
  tmux paste-buffer -d -b "$buffer" -t "$(target_for "$session")"
  if [[ "$press_enter" -eq 1 ]]; then
    tmux send-keys -t "$(target_for "$session")" Enter
  fi
  printf 'sent text to %s\n' "$session"
}

cmd_keys() {
  need_tmux
  parse_session "$@"
  set -- "${remaining[@]}"
  [[ $# -gt 0 ]] || die "keys requires at least one tmux key name"
  session_exists "$session" || die "session not found: $session"
  tmux send-keys -t "$(target_for "$session")" "$@"
  printf 'sent keys to %s: %s\n' "$session" "$*"
}

render_png() {
  local txt="$1"
  local png="$2"
  local out_dir
  out_dir="$(dirname "$txt")"

  if command -v qlmanage >/dev/null 2>&1; then
    qlmanage -t -s 1600 -o "$out_dir" "$txt" >/dev/null 2>&1 || return 1
    if [[ -f "$txt.png" ]]; then
      mv "$txt.png" "$png"
      return 0
    fi
    local generated
    generated="$(find "$out_dir" -maxdepth 1 -type f -name "$(basename "$txt")*.png" -print -quit)"
    if [[ -n "$generated" ]]; then
      mv "$generated" "$png"
      return 0
    fi
  fi

  if python3 - <<'PY' >/dev/null 2>&1
import PIL
PY
  then
    python3 "$SCRIPT_DIR/text_to_png.py" "$txt" "$png"
    return 0
  fi

  return 1
}

cmd_capture() {
  need_tmux
  local session="$DEFAULT_SESSION"
  local out_dir="$DEFAULT_OUT_DIR"
  local lines="$DEFAULT_LINES"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      -s|--session)
        [[ $# -ge 2 ]] || die "missing value for $1"
        session="$2"
        shift 2
        ;;
      -o|--out|--out-dir)
        [[ $# -ge 2 ]] || die "missing value for $1"
        out_dir="$2"
        shift 2
        ;;
      -l|--lines)
        [[ $# -ge 2 ]] || die "missing value for $1"
        lines="$2"
        shift 2
        ;;
      *)
        die "unknown capture argument: $1"
        ;;
    esac
  done

  session_exists "$session" || die "session not found: $session"
  mkdir -p "$out_dir"

  local stamp safe_session txt png
  stamp="$(date +%Y%m%d-%H%M%S)"
  safe_session="$(printf '%s' "$session" | tr -c '[:alnum:]_.-' '-')"
  txt="$out_dir/${safe_session}-${stamp}.txt"
  png="$out_dir/${safe_session}-${stamp}.png"

  tmux capture-pane -t "$(target_for "$session")" -p -J -S "-$lines" > "$txt"
  if render_png "$txt" "$png"; then
    printf 'text: %s\npng:  %s\n' "$txt" "$png"
  else
    printf 'text: %s\npng:  unavailable; install Pillow or run on macOS with qlmanage\n' "$txt"
    return 2
  fi
}

cmd_attach() {
  need_tmux
  parse_session "$@"
  session_exists "$session" || die "session not found: $session"
  tmux attach-session -t "$session"
}

cmd_stop() {
  need_tmux
  parse_session "$@"
  if session_exists "$session"; then
    tmux kill-session -t "$session"
    printf 'stopped %s\n' "$session"
  else
    printf 'session not found: %s\n' "$session"
  fi
}

cmd_status() {
  need_tmux
  parse_session "$@"
  session_exists "$session" || die "session not found: $session"
  tmux list-panes -t "$session" -F '#{session_name}:#{window_index}.#{pane_index} #{pane_current_command} #{pane_current_path} #{pane_width}x#{pane_height}'
}

main() {
  [[ $# -gt 0 ]] || {
    usage
    exit 1
  }

  local subcommand="$1"
  shift
  case "$subcommand" in
    start) cmd_start "$@" ;;
    send) cmd_send "$@" ;;
    keys) cmd_keys "$@" ;;
    capture) cmd_capture "$@" ;;
    attach) cmd_attach "$@" ;;
    stop) cmd_stop "$@" ;;
    status) cmd_status "$@" ;;
    -h|--help|help) usage ;;
    *) die "unknown subcommand: $subcommand" ;;
  esac
}

main "$@"
