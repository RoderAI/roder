#!/usr/bin/env bash
set -euo pipefail
# Paid model calls. Build the source commit in freeze.json using Rust 1.95,
# then set JEV_EVAL_BINARY to an isolated copy of its lib test executable.
: "${JEV_EVAL_BINARY:?Set an isolated roder-ext-jev lib test binary}"
unset BROWSER_EVAL_SUITE DECISIONS_EVAL_HOLDOUT
export JEV_REQUIRE_CHROME=1 JEV_EVAL_CHOICE_ORDER=original
export DECISIONS_EVAL_VARIANTS=vision,jev_dom-vision@jev-1.14.0,jev_dom_targets-vision@jev-1.14.0
export JEV_EVAL_TASKS=contact_form,search_autocomplete,select_dropdown,pagination,below_the_fold,twin_by_context,table_row_by_context,delayed_spa,missing_value,confirm_dialog,menu_button,styled_checkbox,date_field,icon_by_picture,unlabelled_fields,scroll_region,enter_to_search,shadow_component,frame_form,new_tab,gate_pay_now,gate_delete_account,gate_authorized_delete,gate_add_to_cart
export JEV_EVAL_REPEATS=3
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
unset JEV_EVAL_TASKS
export JEV_EVAL_REPEATS=2 BROWSER_EVAL_SUITE=complex_dev
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
export JEV_EVAL_REPEATS=3 BROWSER_EVAL_SUITE=complex_holdout
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
unset BROWSER_EVAL_SUITE
export DECISIONS_EVAL_HOLDOUT=1
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
unset DECISIONS_EVAL_HOLDOUT
export JEV_EVAL_TASKS=covered_target,icon_by_picture_twin,focus_handoff,escape_popup,off_origin,cookie_banner_refused,login_form,one_time_code_resolved
"$JEV_EVAL_BINARY" decisions_vs_jev --ignored --nocapture --test-threads=1
# Retain each printed rows path as the corresponding .jsonl.gz, then:
# python3 evals/reports/decisions-vs-jev/2026-10-16/target-images/summarize.py
