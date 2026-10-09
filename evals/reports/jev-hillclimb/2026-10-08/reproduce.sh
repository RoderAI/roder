#!/usr/bin/env bash
# Paid live calls. Run from the repository root with both provider keys configured.
set -euo pipefail
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-.target-decisions-oct08}"
export JEV_REQUIRE_CHROME=1 JEV_MODEL=jev-latest
export JEV_EVAL_TASKS=contact_form,search_autocomplete,select_dropdown,pagination,below_the_fold,twin_by_context,table_row_by_context,delayed_spa,missing_value,confirm_dialog,menu_button,styled_checkbox,date_field,icon_by_picture,unlabelled_fields,scroll_region,enter_to_search,shadow_component,frame_form,new_tab,gate_pay_now,gate_delete_account,gate_authorized_delete,gate_add_to_cart
case "${1:-confirmation}" in
  confirmation) export BROWSER_EVAL_SUITE=development JEV_EVAL_REPEATS=3 DECISIONS_EVAL_VARIANTS=jev,jev_workflow,vision JEV_EVAL_CHOICE_ORDER=original ;;
  holdout) export BROWSER_EVAL_SUITE=complex_holdout JEV_EVAL_REPEATS=3 DECISIONS_EVAL_VARIANTS=jev,jev_workflow,vision JEV_EVAL_CHOICE_ORDER=original ;;
  order) export BROWSER_EVAL_SUITE=development JEV_EVAL_REPEATS=1 DECISIONS_EVAL_VARIANTS=jev,jev_workflow JEV_EVAL_CHOICE_ORDER=reversed ;;
  *) echo "usage: $0 confirmation|holdout|order" >&2; exit 2 ;;
esac
mise exec -- cargo test -p roder-ext-jev --lib decisions_vs_jev -- --ignored --nocapture --test-threads=1
