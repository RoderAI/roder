#!/usr/bin/env bash
set -euo pipefail

lab_dir="${1:-${RODER_ROUTING_LAB:-/tmp/roder-routing-lab}}"
source_config="${RODER_ROUTING_SOURCE_CONFIG:-$HOME/.roder/config.toml}"
source_auth="${RODER_ROUTING_SOURCE_AUTH:-$HOME/.roder/auth}"

provider="${RODER_ROUTING_PROVIDER:-codex}"
simple_model="${RODER_ROUTING_SIMPLE_MODEL:-gpt-5.3-codex-spark}"
standard_model="${RODER_ROUTING_STANDARD_MODEL:-gpt-5.4-mini}"
strong_model="${RODER_ROUTING_STRONG_MODEL:-gpt-5.5}"

simple_reasoning="${RODER_ROUTING_SIMPLE_REASONING:-low}"
standard_reasoning="${RODER_ROUTING_STANDARD_REASONING:-medium}"
strong_reasoning="${RODER_ROUTING_STRONG_REASONING:-high}"

simple_input_price="${RODER_ROUTING_SIMPLE_INPUT_PRICE:-0.10}"
simple_output_price="${RODER_ROUTING_SIMPLE_OUTPUT_PRICE:-0.40}"
standard_input_price="${RODER_ROUTING_STANDARD_INPUT_PRICE:-0.50}"
standard_output_price="${RODER_ROUTING_STANDARD_OUTPUT_PRICE:-2.00}"
strong_input_price="${RODER_ROUTING_STRONG_INPUT_PRICE:-2.00}"
strong_output_price="${RODER_ROUTING_STRONG_OUTPUT_PRICE:-8.00}"

if [[ "$lab_dir" == "$HOME/.roder" ]]; then
  echo "Refusing to write the lab config over ~/.roder. Choose another directory." >&2
  exit 1
fi

mkdir -p "$lab_dir"

config_path="$lab_dir/config.toml"
tmp_config="$(mktemp)"
cleanup() {
  rm -f "$tmp_config"
}
trap cleanup EXIT

{
  printf 'provider = "%s"\n' "$provider"
  printf 'model = "%s"\n\n' "$strong_model"

  if [[ -f "$source_config" ]]; then
    awk '
      BEGIN { skip = 0; in_root = 1 }
      /^\[inference_router(\]|\.)/ { skip = 1; next }
      /^\[/ { skip = 0; in_root = 0 }
      skip { next }
      in_root && /^[[:space:]]*(provider|model)[[:space:]]*=/ { next }
      { print }
    ' "$source_config"
    printf '\n'
  fi

  cat <<ROUTING_CONFIG
[inference_router]
enabled = true
router = "local"
profile = "coding"
baseline_provider = "$provider"
baseline_model = "$strong_model"

[inference_router.extension]
objective = "cost"

[inference_router.extension.tiers.simple]
provider = "$provider"
model = "$simple_model"
reasoning = "$simple_reasoning"

[inference_router.extension.tiers.standard]
provider = "$provider"
model = "$standard_model"
reasoning = "$standard_reasoning"

[inference_router.extension.tiers.strong]
provider = "$provider"
model = "$strong_model"
reasoning = "$strong_reasoning"

[inference_router.extension.profiles.coding]
default_tier = "standard"
simple_tier = "simple"
standard_tier = "standard"
strong_tier = "strong"
risk_floor_tier = "strong"
classifier_prompt = "Reserved for future classifier comparison."

[inference_router.extension.profiles.coding.risk_floors]
security = "strong"
privacy = "strong"
data_loss = "strong"
infra = "strong"
architecture = "strong"

[inference_router.extension.prices."$provider/$simple_model"]
input_per_million = $simple_input_price
output_per_million = $simple_output_price

[inference_router.extension.prices."$provider/$standard_model"]
input_per_million = $standard_input_price
output_per_million = $standard_output_price

[inference_router.extension.prices."$provider/$strong_model"]
input_per_million = $strong_input_price
output_per_million = $strong_output_price
ROUTING_CONFIG
} >"$tmp_config"

mv "$tmp_config" "$config_path"

if [[ -d "$source_auth" ]]; then
  rm -rf "$lab_dir/auth"
  cp -R "$source_auth" "$lab_dir/auth"
fi

cat <<EOF
Prepared inference-routing lab config:
  $config_path

Run the TUI:
  cargo run -p roder-cli -- --config-dir "$lab_dir"

In another terminal, watch routing decisions:
  scripts/roder-inference-routing-lab-tail.sh "$lab_dir" --follow

After a turn finishes, inspect status and metrics:
  scripts/roder-inference-routing-lab-metrics.sh "$lab_dir"
EOF
