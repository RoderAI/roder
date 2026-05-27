import ast
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RUN_SCRIPT = ROOT / "evals/harbor/tbench_campaign_run_script.py"

COMMAND_HELPERS = {
    "array_literal_values",
    "array_append_flag_values",
    "analysis_command_tuples",
    "baseline_validation_command_tuples",
    "campaign_validation_command_tuples",
    "command_flag_values",
    "expected_analysis_tuples",
    "expected_baseline_validation_tuples",
    "expected_campaign_validation_tuples",
    "expected_image_preflight_tuples",
    "expected_route_job_dirs",
    "format_tuple",
    "has_flag_value",
    "image_preflight_command_tuples",
    "int_value",
    "route_job_dir_values",
    "script_flag_values",
    "validate_final_campaign_validation_order",
    "validate_route_command_order",
}


class TbenchCampaignRunScriptStructureTest(unittest.TestCase):
    def test_command_helpers_are_imported_from_dedicated_module(self) -> None:
        tree = ast.parse(RUN_SCRIPT.read_text())
        imported = {
            alias.name
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == "tbench_campaign_script_commands"
            for alias in node.names
        }
        defined = {
            node.name
            for node in tree.body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }

        self.assertEqual(COMMAND_HELPERS, imported.intersection(COMMAND_HELPERS))
        self.assertFalse(
            COMMAND_HELPERS.intersection(defined),
            "command helpers should live in tbench_campaign_script_commands.py",
        )


if __name__ == "__main__":
    unittest.main()
