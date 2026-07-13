#!/usr/bin/env python3
"""Fail-closed tests for the checked dogfood fixture contract."""

from __future__ import annotations

import copy
import importlib.machinery
import importlib.util
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/dogfood-oracle"
loader = importlib.machinery.SourceFileLoader("dogfood_oracle", str(SCRIPT))
spec = importlib.util.spec_from_loader(loader.name, loader)
assert spec is not None
module = importlib.util.module_from_spec(spec)
loader.exec_module(module)
CONTRACT = json.loads((ROOT / "tests/dogfood-suite/contract.json").read_text())


class ContractTest(unittest.TestCase):
    def test_checked_contract_is_valid_and_render_is_idempotent(self) -> None:
        module.validate(CONTRACT)
        rendered = module.render(CONTRACT)
        self.assertEqual(rendered, module.render(json.loads(json.dumps(CONTRACT))))
        self.assertEqual(rendered, (ROOT / "DOGFOOD_SUITE.md").read_text())

    def assert_rejected(self, mutate) -> None:
        contract = copy.deepcopy(CONTRACT)
        mutate(contract)
        with self.assertRaises(ValueError):
            module.validate(contract)

    def test_missing_package_fails_closed(self) -> None:
        self.assert_rejected(lambda value: value["packages"].pop())

    def test_stale_source_tree_fails_closed(self) -> None:
        self.assert_rejected(
            lambda value: value["packages"][0]["source"].__setitem__("tree", "0" * 40)
        )

    def test_duplicate_case_fails_closed(self) -> None:
        def mutate(value) -> None:
            value["packages"][1]["cases"][0]["id"] = value["packages"][0]["cases"][0]["id"]

        self.assert_rejected(mutate)

    def test_missing_cleanup_fails_closed(self) -> None:
        self.assert_rejected(lambda value: value["packages"][0].__setitem__("cleanup", []))

    def test_missing_fixture_kind_fails_closed(self) -> None:
        def mutate(value) -> None:
            for package in value["packages"]:
                for case in package["cases"]:
                    case["kinds"] = [kind for kind in case["kinds"] if kind != "browser_socket"]

        self.assert_rejected(mutate)

    def test_bundle_drift_fails_closed(self) -> None:
        self.assert_rejected(lambda value: value["bundles"]["default"].append("morph"))


if __name__ == "__main__":
    unittest.main()
