#!/usr/bin/env python3
"""Negative controls for the first-party construction inventory checker."""

from __future__ import annotations

import json
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts/construction-inventory"


class ConstructionInventoryCheckerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for relative in (
            "PLAN.md",
            "CONSTRUCTION_INVENTORY.md",
            "tests/construction-inventory/provenance.json",
            "tests/construction-inventory/manifest.json",
            "crates/pi-rs-agent/src/lib.rs",
            "crates/pi-rs-agent/lua",
            "crates/pi-rs-app/src",
            "crates/pi-rs-host/src/lib.rs",
        ):
            source = ROOT / relative
            destination = self.root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, destination)
            else:
                shutil.copy2(source, destination)
        for path in self.root.rglob("*"):
            path.chmod(path.stat().st_mode | stat.S_IWUSR)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_checker(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(CHECKER), "--root", str(self.root), *args],
            text=True,
            capture_output=True,
            check=False,
        )

    def manifest(self) -> dict:
        return json.loads((self.root / "tests/construction-inventory/manifest.json").read_text())

    def write_manifest(self, manifest: dict) -> None:
        (self.root / "tests/construction-inventory/manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n"
        )

    def assert_rejected(self, expected: str) -> None:
        result = self.run_checker("--check")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(expected, result.stderr)

    def test_generation_is_byte_idempotent(self) -> None:
        before = (self.root / "CONSTRUCTION_INVENTORY.md").read_bytes()
        first = self.run_checker()
        self.assertEqual(first.returncode, 0, first.stderr)
        middle = (self.root / "CONSTRUCTION_INVENTORY.md").read_bytes()
        second = self.run_checker()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(before, middle)
        self.assertEqual(middle, (self.root / "CONSTRUCTION_INVENTORY.md").read_bytes())
        checked = self.run_checker("--check")
        self.assertEqual(checked.returncode, 0, checked.stderr)

    def test_unclassified_embedded_source_is_rejected(self) -> None:
        new_source = self.root / "crates/pi-rs-app/src/builtins/tools/new-policy.lua"
        new_source.write_text("local new_policy = true\n")
        descriptor = self.root / "crates/pi-rs-app/src/builtins/mod.rs"
        source = descriptor.read_text()
        source = source.replace(
            'include_str!("tools/prelude.lua"),',
            'include_str!("tools/new-policy.lua"),\n        include_str!("tools/prelude.lua"),',
            1,
        )
        descriptor.write_text(source)
        self.assert_rejected("embedded source coverage differs")

    def test_unclassified_public_declaration_is_rejected(self) -> None:
        frontend = self.root / "crates/pi-rs-app/src/builtins/interactive.lua"
        frontend.write_text(
            frontend.read_text()
            + '\npi.register_command("unclassified-policy", { handler = function() end })\n'
        )
        self.assert_rejected("embedded declarations differ")

    def test_duplicate_declaration_owner_is_rejected(self) -> None:
        manifest = self.manifest()
        row = next(row for row in manifest["rows"] if row["id"] == "tool.bash")
        row["declarations"].append("tool:read")
        self.write_manifest(manifest)
        self.assert_rejected("duplicates=")

    def test_stale_source_row_is_rejected(self) -> None:
        manifest = self.manifest()
        row = next(row for row in manifest["rows"] if row["id"] == "tool.read")
        row["coverage"] = ["crates/pi-rs-app/src/builtins/tools/removed.lua"]
        self.write_manifest(manifest)
        self.assert_rejected("stale=")

    def test_hardcoded_product_entrypoint_is_rejected(self) -> None:
        main = self.root / "crates/pi-rs-app/src/main.rs"
        main.write_text(main.read_text() + '\nconst BAD: &str = "pi-rs-run";\n')
        self.assert_rejected("hardcoded Rust product entrypoints differ")

    def test_stale_rust_seam_is_rejected(self) -> None:
        main = self.root / "crates/pi-rs-app/src/main.rs"
        main.write_text(main.read_text().replace("let role = if interactive", "let selected_role = if interactive", 1))
        self.assert_rejected("stale anchor")

    def test_unclassified_rust_launch_call_is_rejected(self) -> None:
        main = self.root / "crates/pi-rs-app/src/main.rs"
        main.write_text(main.read_text() + "\n// inventory negative control: host.call_command(name, args);\n")
        self.assert_rejected("Rust launch/composition calls differ")

    def test_missing_named_open_row_is_rejected(self) -> None:
        manifest = self.manifest()
        manifest["rows"] = [
            row for row in manifest["rows"] if row["id"] != "dogfood.pi-compact-renderer-patching"
        ]
        self.write_manifest(manifest)
        self.assert_rejected("missing named open rows")


if __name__ == "__main__":
    unittest.main()
