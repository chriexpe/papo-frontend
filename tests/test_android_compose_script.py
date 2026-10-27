"""Exercise the Compose build entry point without an SDK, device, or ripgrep."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/android-compose.sh"


class ComposeBuildScriptTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="papo android release ")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "scripts").mkdir()
        shutil.copyfile(SCRIPT, self.root / "scripts/android-compose.sh")
        (self.root / "scripts/prepare-winit.py").write_text("pass\n")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        # Deliberately exclude the host PATH: rg must not be required.
        for tool in ["bash", "dirname", "grep"]:
            (self.bin / tool).symlink_to(shutil.which(tool))
        (self.bin / "python3").symlink_to(sys.executable)
        self.write_program(
            self.bin / "rustup",
            'printf "%s\\n" "$PAPO_SCRIPT_TARGETS"\n',
        )
        self.write_program(
            self.root / "android/gradlew",
            'printf "%s\\n" "$*" >> "$PAPO_SCRIPT_GRADLE_LOG"\n',
        )
        self.write_program(
            self.root / "sdk/platform-tools/adb",
            'printf "%s\\n" "$*" >> "$PAPO_SCRIPT_ADB_LOG"\n'
            'case "$*" in *"am instrument"*) '
            'printf "%s\\n" "$PAPO_SCRIPT_INSTRUMENTATION";; esac\n',
        )
        for variant in ["debug", "release"]:
            apk = self.root / f"android/compose/build/outputs/apk/{variant}/compose-{variant}.apk"
            apk.parent.mkdir(parents=True)
            apk.write_bytes(b"fixture")
        self.env = dict(
            os.environ,
            PATH=str(self.bin),
            ANDROID_HOME=str(self.root / "sdk"),
            PAPO_ANDROID_ABI="arm64-v8a",
            PAPO_SCRIPT_TARGETS="aarch64-linux-android\nx86_64-linux-android",
            PAPO_SCRIPT_GRADLE_LOG=str(self.root / "gradle.log"),
            PAPO_SCRIPT_ADB_LOG=str(self.root / "adb.log"),
            PAPO_SCRIPT_INSTRUMENTATION="OK (2 tests)",
        )

    def write_program(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\nset -eu\n" + body)
        path.chmod(0o755)

    def run_script(self, *args):
        return subprocess.run(
            [str(self.bin / "bash"), str(self.root / "scripts/android-compose.sh"), *args],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_release_builds_compose_and_launches_official_application_without_rg(self):
        result = self.run_script("--arm64", "--release", "--run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(":compose:assembleRelease", (self.root / "gradle.log").read_text())
        self.assertIn("-PpapoAbis=arm64-v8a", (self.root / "gradle.log").read_text())
        self.assertIn("outputs/apk/release/compose-release.apk", result.stdout)
        self.assertIn(
            "am start -n io.github.chriexpe.papo/io.github.chriexpe.papo.compose.MainActivity",
            (self.root / "adb.log").read_text(),
        )

    def test_debug_keeps_its_separate_application_and_emulator_abi(self):
        result = self.run_script("--x86_64", "--run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(":compose:assembleDebug", (self.root / "gradle.log").read_text())
        self.assertIn("-PpapoAbis=x86_64", (self.root / "gradle.log").read_text())
        self.assertIn(
            "am start -n io.github.chriexpe.papo.compose/io.github.chriexpe.papo.compose.MainActivity",
            (self.root / "adb.log").read_text(),
        )

    def test_missing_target_fails_before_gradle(self):
        self.env["PAPO_SCRIPT_TARGETS"] = "x86_64-linux-android"
        result = self.run_script("--arm64", "--release")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("rustup target add aarch64-linux-android", result.stderr)
        self.assertFalse((self.root / "gradle.log").exists())

    def test_instrumentation_still_requires_a_nonzero_pass_count(self):
        self.env["PAPO_SCRIPT_INSTRUMENTATION"] = "OK (0 tests)"
        result = self.run_script("--x86_64", "--test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("nonzero passing test count", result.stderr)

    def test_instrumentation_accepts_success_without_rg(self):
        result = self.run_script("--x86_64", "--test")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("OK (2 tests)", result.stdout)


if __name__ == "__main__":
    unittest.main()
