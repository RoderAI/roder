#!/usr/bin/env bash
set -euo pipefail
# Configure OPENAI_API_KEY and JEV_API_KEY (or their Roder provider entries).
# These tests make paid API calls. Run from the repository root.
export JEV_EVAL_TASKS=contact_form,search_autocomplete,select_dropdown,pagination,below_the_fold,twin_by_context,table_row_by_context,delayed_spa,missing_value,confirm_dialog,menu_button,styled_checkbox,date_field,icon_by_picture,unlabelled_fields,scroll_region,enter_to_search,shadow_component,frame_form,new_tab,gate_pay_now,gate_delete_account,gate_authorized_delete,gate_add_to_cart
export JEV_EVAL_REPEATS=3 JEV_REQUIRE_CHROME=1 JEV_MODEL=jev-latest
export DECISIONS_EVAL_VARIANTS=original,effects,vision,jev
mise exec -- cargo test -p roder-ext-jev --lib decisions_vs_jev -- --ignored --nocapture --test-threads=1
export DECISIONS_EVAL_HOLDOUT=1
mise exec -- cargo test -p roder-ext-jev --lib decisions_vs_jev -- --ignored --nocapture --test-threads=1
