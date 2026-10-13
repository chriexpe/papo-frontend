"""Preparation regression tests; no registry writes or global Git changes."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
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
            encoding="utf-8", newline="\n",
        )
        (self.root / "Cargo.lock").write_bytes(b"lockfile must stay unchanged\n")
        self.patch_file = self.root / prepare.PATCH
        self.write_patch(2)
        self.real_run = subprocess.run
        self.real_which = shutil.which
        self.calls = 0
        self.failure = False
        self.partial = False
        self.version = prepare.TOOL_VERSION
        self.child_env = None
        self.run_mock = patch.object(prepare.subprocess, "run", side_effect=self.tool_run)
        self.run_mock.start()
        self.addCleanup(self.run_mock.stop)
        which = patch.object(prepare.shutil, "which", side_effect=self.which)
        which.start()
        self.addCleanup(which.stop)
        archive = patch.object(prepare, "verify_archive")
        self.archive_check = archive.start()
        self.addCleanup(archive.stop)

    def which(self, name):
        return "fixture-tool" if name == "cargo-patch-crate" else self.real_which(name)

    def write_patch(self, value):
        self.patch_file.write_text(
            f"--- a/fixture.rs\n+++ b/fixture.rs\n@@ -1 +1 @@\n-const VALUE: u32 = 1;\n+const VALUE: u32 = {value};\n",
            encoding="utf-8", newline="\n",
        )

    def tool_run(self, args, **kwargs):
        if args[0] != "fixture-tool":
            return self.real_run(args, **kwargs)
        if args[1] == "--version":
            return subprocess.CompletedProcess(args, 0, f"patch-crate {self.version}\n", "")
        self.calls += 1
        self.child_env = kwargs["env"]
        crate = self.root / prepare.DESTINATION
        if crate.exists():
            shutil.rmtree(crate)
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text(
            f'[package]\nname="winit"\nversion="{prepare.VERSION}"\n', encoding="utf-8", newline="\n"
        )
        (crate / "fixture.rs").write_text("const VALUE: u32 = 1;\n", encoding="utf-8", newline="\n")
        if self.failure:
            raise subprocess.CalledProcessError(1, args)
        if not self.partial:
            self.real_run(["git", "apply", str(self.patch_file)], cwd=crate, check=True)
        return subprocess.CompletedProcess(args, 0)

    def test_clean_preparation_is_verified_and_does_not_change_lockfile(self):
        original = (self.root / "Cargo.lock").read_bytes()
        prepare.prepare(self.root)
        self.assertTrue(prepare.prepared(self.root))
        self.assertEqual(original, (self.root / "Cargo.lock").read_bytes())
        self.archive_check.assert_called_once_with(self.root)

    def test_verified_preparation_is_reused_and_force_recreates_it(self):
        prepare.prepare(self.root)
        prepare.prepare(self.root)
        self.assertEqual(self.calls, 1)
        prepare.prepare(self.root, force=True)
        self.assertEqual(self.calls, 2)
        self.assertTrue(prepare.prepared(self.root))

    def test_changed_patch_rebuilds_instead_of_silently_using_cached_source(self):
        prepare.prepare(self.root)
        self.write_patch(3)
        self.assertFalse(prepare.prepared(self.root))
        prepare.prepare(self.root)
        self.assertEqual(self.calls, 2)
        self.assertEqual((self.root / prepare.DESTINATION / "fixture.rs").read_text(), "const VALUE: u32 = 3;\n")

    def test_corrupted_source_is_detected_even_with_a_matching_marker(self):
        prepare.prepare(self.root)
        (self.root / prepare.DESTINATION / "fixture.rs").write_text("wrong content\n")
        self.assertFalse(prepare.prepared(self.root))
        prepare.prepare(self.root)
        self.assertEqual(self.calls, 2)
        self.assertTrue(prepare.prepared(self.root))

    def test_failed_or_incomplete_application_removes_the_generated_crate(self):
        for partial in [False, True]:
            self.failure = not partial
            self.partial = partial
            with self.assertRaises((prepare.PreparationError, subprocess.CalledProcessError)):
                prepare.prepare(self.root)
            self.assertFalse((self.root / prepare.DESTINATION).exists())

    def test_wrong_archive_is_not_published_as_a_prepared_crate(self):
        self.archive_check.side_effect = prepare.PreparationError("wrong SHA-256")
        with self.assertRaisesRegex(prepare.PreparationError, "SHA-256"):
            prepare.prepare(self.root)
        self.assertFalse((self.root / prepare.DESTINATION).exists())

    def test_missing_tool_and_wrong_version_fail_with_install_instructions(self):
        with patch.object(prepare.shutil, "which", side_effect=lambda name: None if name == "cargo-patch-crate" else self.real_which(name)):
            with self.assertRaisesRegex(prepare.PreparationError, "cargo install patch-crate"):
                prepare.prepare(self.root)
        self.version = "0.1.12"
        with self.assertRaisesRegex(prepare.PreparationError, "0.1.13"):
            prepare.prepare(self.root)
        self.assertEqual(self.calls, 0)

    def test_checkpoint_git_settings_are_local_to_the_tool_process(self):
        with patch.dict(os.environ, {
            "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.autocrlf", "GIT_CONFIG_VALUE_0": "false",
        }):
            before = dict(os.environ)
            prepare.prepare(self.root)
            self.assertEqual(before, dict(os.environ))
        env = self.child_env
        self.assertEqual(env["GIT_CONFIG_COUNT"], "4")
        self.assertEqual(env["GIT_CONFIG_KEY_0"], "core.autocrlf")
        self.assertEqual(env["GIT_CONFIG_KEY_1"], "commit.gpgsign")
        self.assertEqual(env["GIT_CONFIG_VALUE_1"], "false")
        self.assertEqual(env["GIT_AUTHOR_EMAIL"], "patches@papo.invalid")

    def test_check_only_does_not_prepare_and_parallel_preparation_is_rejected(self):
        with self.assertRaises(prepare.PreparationError):
            prepare.prepare(self.root, check_only=True)
        self.assertEqual(self.calls, 0)
        with prepare.preparation_lock(self.root):
            with self.assertRaisesRegex(prepare.PreparationError, "Another"):
                prepare.prepare(self.root)


class ArchiveTests(unittest.TestCase):
    def test_archive_checksum_and_relative_cargo_home(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache = root / "cargo-home/registry/cache/fixture"
            cache.mkdir(parents=True)
            archive = cache / f"winit-{prepare.VERSION}.crate"
            archive.write_bytes(b"original archive")
            with patch.dict(os.environ, {"CARGO_HOME": "cargo-home"}), patch.object(
                prepare, "CHECKSUM", hashlib.sha256(archive.read_bytes()).hexdigest()
            ):
                prepare.verify_archive(root)
                archive.write_bytes(b"corrupted archive")
                with self.assertRaisesRegex(prepare.PreparationError, "SHA-256"):
                    prepare.verify_archive(root)
                archive.unlink()
                with self.assertRaises(prepare.PreparationError):
                    prepare.verify_archive(root)


if __name__ == "__main__":
    unittest.main()
