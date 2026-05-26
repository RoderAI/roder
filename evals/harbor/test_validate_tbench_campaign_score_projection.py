#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

try:
    from .tbench_campaign_test_helpers import generate_campaign, validate_campaign
except ImportError:
    from tbench_campaign_test_helpers import generate_campaign, validate_campaign


class ValidateTbenchCampaignScoreProjectionTests(unittest.TestCase):
    def test_rejects_stale_score_projection(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp) / "campaign")
            data = json.loads(manifest.read_text())
            data["scoreProjection"] = {
                "suiteTasks": 89,
                "baselinePasses": 50,
                "campaignConversionCandidates": 7,
                "projectedPassesIfAllRoutesPass": 99,
                "projectedMeanIfAllRoutesPass": 99 / 89,
                "codexCliTargetPasses": 73,
                "codexCliGap": 0,
                "sotaTargetPasses": 76,
                "sotaGap": 0,
            }
            manifest.write_text(json.dumps(data, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "scoreProjection projectedPassesIfAllRoutesPass mismatch",
            result.stderr,
        )


if __name__ == "__main__":
    unittest.main()
