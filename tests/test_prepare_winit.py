"""Regression tests for reproducible, no-Rust-CLI winit patch preparation."""

import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "prepare_winit", Path(__file__).resolve().parents[1] / "scripts/prepare-winit.py"
)
prepare = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prepare)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "patches").mkdir()
        (self.root / "Cargo.toml").write_text(
            '[package]\nname="fixture"\nversion="0.1.0"\n'
            '[package.metadata.patch]\ncrates=["winit"]\n'
            f'[target.\'cfg(target_os = "linux")\'.dependencies]\nwinit={{version="={prepare.VERSION}"}}\n'
            f'[patch.crates-io]\nwinit={{path="{prepare.DESTINATION.as_posix()}"}}\n',
            encoding="utf-8", newline="\n"
        )
        (self.root / "Cargo.lock").write_bytes(b"locked\n")
        self.patch_file = self.root / prepare.PATCH
        self.write_patch(2)
        self.archive = self.make_archive()
        self.checksum = hashlib.sha256(self.archive).hexdigest()
        self.checksum_patch = patch.object(prepare, "CHECKSUM", self.checksum)
        self.checksum_patch.start()
        self.addCleanup(self.checksum_patch.stop)
        self.archive_patch = patch.object(prepare, "archive_bytes", return_value=self.archive)
        self.archive_mock = self.archive_patch.start()
        self.addCleanup(self.archive_patch.stop)

    def write_patch(self, value):
        self.patch_file.write_text(
            f"--- a/fixture.rs\n+++ b/fixture.rs\n@@ -1 +1 @@\n-const VALUE: u32 = 1;\n+const VALUE: u32 = {value};\n",
            encoding="utf-8", newline="\n",
        )

    def make_archive(self, unsafe_name=None):
        content = io.BytesIO()
        with tarfile.open(fileobj=content, mode="w:gz") as archive:
            prefix = f"winit-{prepare.VERSION}"
            files = {
                f"{prefix}/Cargo.toml": f'[package]\nname="winit"\nversion="{prepare.VERSION}"\n',
                f"{prefix}/fixture.rs": "const VALUE: u32 = 1;\n",
            }
            if unsafe_name:
                files[unsafe_name] = "malicious\n"
            for filename, text in files.items():
                payload = text.encode()
                info = tarfile.TarInfo(filename)
                info.size = len(payload)
                archive.addfile(info, io.BytesIO(payload))
        return content.getvalue()

    def test_prepare_is_idempotent_and_locked(self):
        prepare.prepare(self.root)
        self.assertTrue(prepare.prepared(self.root))
        prepare.prepare(self.root)
        self.archive_mock.assert_called_once()
        self.assertEqual((self.root / "Cargo.lock").read_bytes(), b"locked\n")

    def test_force_and_changed_diff_recreate_source(self):
        prepare.prepare(self.root)
        prepare.prepare(self.root, force=True)
        self.assertEqual(self.archive_mock.call_count, 2)
        self.write_patch(3)
        self.assertFalse(prepare.prepared(self.root))
        prepare.prepare(self.root)
        self.assertEqual((self.root / prepare.DESTINATION / "fixture.rs").read_text(), "const VALUE: u32 = 3;\n")

    def test_corruption_of_patched_file_is_detected(self):
        prepare.prepare(self.root)
        (self.root / prepare.DESTINATION / "fixture.rs").write_text("wrong content\n")
        self.assertFalse(prepare.prepared(self.root))
        prepare.prepare(self.root)
        self.assertTrue(prepare.prepared(self.root))

    def test_bad_checksum_cannot_be_published(self):
        self.archive_patch.stop()
        with patch.object(prepare, "archive_bytes", side_effect=prepare.PreparationError("wrong SHA-256")):
            with self.assertRaisesRegex(prepare.PreparationError, "SHA-256"):
                prepare.prepare(self.root)
        self.assertFalse((self.root / prepare.DESTINATION).exists())

    def test_missing_and_unsafe_archive_files(self):
        with self.assertRaises(prepare.PreparationError):
            prepare.unpack_archive(self.make_archive("../outside"), self.root)
        self.assertFalse((self.root / "outside").exists())

    def test_invalid_diff_rejects_unchanged_destination(self):
        prepare.prepare(self.root)
        old = (self.root / prepare.DESTINATION / "fixture.rs").read_bytes()
        self.write_patch(999)
        self.patch_file.write_text("not a patch")
        with self.assertRaises(prepare.PreparationError):
            prepare.prepare(self.root)
        self.assertEqual((self.root / prepare.DESTINATION / "fixture.rs").read_bytes(), old)

    def test_check_only_and_preparation_lock(self):
        with self.assertRaisesRegex(prepare.PreparationError, "before Cargo"):
            prepare.prepare(self.root, check_only=True)
        with prepare.preparation_lock(self.root):
            with self.assertRaisesRegex(prepare.PreparationError, "Another"):
                prepare.prepare(self.root)

    def test_source_outside_target_tree(self):
        self.assertNotEqual(prepare.DESTINATION.parts[0], "target")
        prepare.prepare(self.root)
        self.assertFalse((self.root / "target").exists())

    def test_archive_checksum(self):
        self.assertEqual(prepare.verify_archive_bytes(self.archive), self.archive)
        with self.assertRaisesRegex(prepare.PreparationError, "SHA-256"):
            prepare.verify_archive_bytes(b"tampered")

    def test_relative_cargo_home(self):
        self.archive_patch.stop()
        cache = self.root / "cargo-home/registry/cache/fixture"
        cache.mkdir(parents=True)
        (cache / f"winit-{prepare.VERSION}.crate").write_bytes(self.archive)
        with patch.dict(os.environ, {"CARGO_HOME": "cargo-home"}):
            self.assertEqual(prepare.archive_bytes(self.root), self.archive)


if __name__ == "__main__":
    unittest.main()
