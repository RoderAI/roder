import ast
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
AGENT = ROOT / "evals/harbor/roder_harbor_agent.py"

CONFIG_HELPERS = {
    "optional_bool",
    "optional_float",
    "optional_int",
    "optional_int_list",
    "reliability_config_toml",
    "speed_policy_config_toml",
}

REMOVED_METHODS = {
    "_optional_bool",
    "_optional_float",
    "_optional_int",
    "_optional_int_list",
    "_reliability_config_toml",
    "_speed_policy_config_toml",
    "_toml_value",
}


class RoderHarborAgentStructureTests(unittest.TestCase):
    def test_config_helpers_are_imported_from_dedicated_module(self) -> None:
        tree = ast.parse(AGENT.read_text())
        imported = {
            alias.name
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == "roder_harbor_agent_config"
            for alias in node.names
        }
        defined = {
            item.name
            for node in tree.body
            if isinstance(node, ast.ClassDef) and node.name == "RoderCli"
            for item in node.body
            if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef))
        }

        self.assertEqual(CONFIG_HELPERS, imported.intersection(CONFIG_HELPERS))
        self.assertFalse(
            REMOVED_METHODS.intersection(defined),
            "config helpers should live in roder_harbor_agent_config.py",
        )


if __name__ == "__main__":
    unittest.main()
