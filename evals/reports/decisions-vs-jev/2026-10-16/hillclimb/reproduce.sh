#!/usr/bin/env bash
set -euo pipefail
# Paid calls; credentials come from environment or Roder provider config.
# Build once, then copy the test executable out of the shared target directory.
# Run only that copy so another worktree cannot replace the running test binary.
unset BROWSER_EVAL_SUITE DECISIONS_EVAL_HOLDOUT
export JEV_EVAL_CHOICE_ORDER=original
export JEV_REQUIRE_CHROME=1
export DECISIONS_EVAL_VARIANTS=vision,jev-vision@jev-preview,jev_dom-vision@jev-preview
export JEV_EVAL_TASKS=contact_form,search_autocomplete,select_dropdown,pagination,below_the_fold,twin_by_context,table_row_by_context,delayed_spa,missing_value,confirm_dialog,menu_button,styled_checkbox,date_field,icon_by_picture,unlabelled_fields,scroll_region,enter_to_search,shadow_component,frame_form,new_tab,gate_pay_now,gate_delete_account,gate_authorized_delete,gate_add_to_cart
export JEV_EVAL_REPEATS=3
: "${JEV_EVAL_BINARY:?Set to an isolated copy of the compiled roder-ext-jev lib test binary}"
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
unset JEV_EVAL_TASKS
export JEV_EVAL_REPEATS=2 BROWSER_EVAL_SUITE=complex_dev
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
export JEV_EVAL_REPEATS=3 BROWSER_EVAL_SUITE=complex_holdout
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
unset BROWSER_EVAL_SUITE
export DECISIONS_EVAL_HOLDOUT=1
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
# Retain the printed JSONL paths as the four named .jsonl.gz artifacts, then:
# python3 evals/reports/decisions-vs-jev/2026-10-16/hillclimb/summarize.py
