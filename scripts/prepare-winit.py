#!/usr/bin/env python3
"""Prepare the versioned winit diff without modifying Cargo's registry cache."""

import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

VERSION = "0.30.13"
TOOL_VERSION = "0.1.13"
CHECKSUM = "a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d"
PATCH = Path(f"patches/winit+{VERSION}.patch")
DESTINATION = Path(f"target/patch/winit-{VERSION}")
MARKER = ".papo-applied.patch"
INSTALL = f"cargo install patch-crate --version {TOOL_VERSION} --locked"


class PreparationError(Exception):
    pass


def run_git(crate, patch, *flags):
    return subprocess.run(
        ["git", "apply", *flags, str(patch)],
        cwd=crate,
        capture_output=True,
        text=True,
        check=False,
    )


def validate_manifest(root):
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    if manifest["package"].get("metadata", {}).get("patch", {}).get("crates") != ["winit"]:
        raise PreparationError('Expected package.metadata.patch.crates = ["winit"].')
    if manifest["patch"]["crates-io"]["winit"] != {"path": DESTINATION.as_posix()}:
        raise PreparationError(f"Expected winit override at {DESTINATION.as_posix()}.")
    dependencies = [
        target.get("dependencies", {}).get("winit")
        for target in manifest.get("target", {}).values()
    ]
    if not any(dep and dep.get("version") == f"={VERSION}" for dep in dependencies):
        raise PreparationError(f"The direct winit dependency must be pinned to ={VERSION}.")


def verify(crate, patch):
    manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
    package = manifest["package"]
    if package.get("name") != "winit" or package.get("version") != VERSION:
        raise PreparationError(f"Expected the published winit {VERSION} package.")
    checksum = json.loads((crate / ".cargo-checksum.json").read_text(encoding="utf-8"))
    if checksum.get("package") != CHECKSUM:
        raise PreparationError("The original winit package checksum does not match.")
    result = run_git(crate, patch, "--reverse", "--check")
    if result.returncode:
        raise PreparationError(f"The winit patch is not fully applied:\n{result.stderr.strip()}")


def prepared(root):
    crate, patch = root / DESTINATION, root / PATCH
    try:
        if (crate / MARKER).read_bytes() != patch.read_bytes():
            return False
        verify(crate, patch)
        return True
    except (OSError, ValueError, KeyError, PreparationError):
        return False


@contextlib.contextmanager
def preparation_lock(root):
    target = root / "target"
    target.mkdir(exist_ok=True)
    with (target / ".prepare-winit.lock").open("a+b") as lock:
        lock.seek(0, os.SEEK_END)
        if lock.tell() == 0:
            lock.write(b"0")
            lock.flush()
        lock.seek(0)
        try:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise PreparationError("Another winit preparation is running.") from error
        try:
            yield
        finally:
            lock.seek(0)
            if os.name == "nt":
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def tool_environment():
    env = os.environ.copy()
    count = int(env.get("GIT_CONFIG_COUNT", "0"))
    settings = (
        ("commit.gpgsign", "false"),
        ("user.name", "Papo patch preparation"),
        ("user.email", "patches@papo.invalid"),
    )
    for offset, (key, value) in enumerate(settings, count):
        env[f"GIT_CONFIG_KEY_{offset}"] = key
        env[f"GIT_CONFIG_VALUE_{offset}"] = value
    env["GIT_CONFIG_COUNT"] = str(count + len(settings))
    # Explicit identities also cover inherited author/committer environment variables.
    for role in ("AUTHOR", "COMMITTER"):
        env[f"GIT_{role}_NAME"] = "Papo patch preparation"
        env[f"GIT_{role}_EMAIL"] = "patches@papo.invalid"
    return env


def find_tool():
    tool = shutil.which("cargo-patch-crate")
    if not tool:
        raise PreparationError(f"patch-crate is required. Install it with:\n  {INSTALL}")
    result = subprocess.run([tool, "--version"], capture_output=True, text=True, check=False)
    if result.returncode or result.stdout.strip() != f"patch-crate {TOOL_VERSION}":
        raise PreparationError(f"patch-crate {TOOL_VERSION} is required. Run:\n  {INSTALL}")
    return tool


def verify_archive(root):
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).expanduser()
    if not cargo_home.is_absolute():
        cargo_home = root / cargo_home
    archives = (cargo_home / "registry/cache").glob(f"*/winit-{VERSION}.crate")
    if not any(hashlib.sha256(path.read_bytes()).hexdigest() == CHECKSUM for path in archives):
        raise PreparationError("The cached original winit archive is missing or has the wrong SHA-256.")


def prepare(root, force=False, check_only=False):
    root = root.resolve()
    validate_manifest(root)
    patch = root / PATCH
    if not patch.is_file():
        raise PreparationError(f"Missing patch: {patch}")
    if not shutil.which("git"):
        raise PreparationError("Git is required to validate and apply the winit patch.")
    with preparation_lock(root):
        if prepared(root) and not force:
            print(f"winit {VERSION}: patch verified.")
            return
        if check_only:
            raise PreparationError("Run python3 scripts/prepare-winit.py before Cargo.")
        crate = root / DESTINATION
        # Never leave an old marker behind if the pinned tool is unavailable.
        (crate / MARKER).unlink(missing_ok=True)
        tool = find_tool()
        lockfile = root / "Cargo.lock"
        before = lockfile.read_bytes() if lockfile.exists() else None
        try:
            subprocess.run([tool, "--force"], cwd=root, env=tool_environment(), check=True)
            if (lockfile.read_bytes() if lockfile.exists() else None) != before:
                raise PreparationError("Cargo.lock changed during preparation; stop concurrent Cargo commands.")
            verify_archive(root)
            # Current Cargo caches omit this file; keep the same provenance
            # record as the checksum-verified Flatpak source.
            (crate / ".cargo-checksum.json").write_text(
                json.dumps({"package": CHECKSUM, "files": {}}), encoding="utf-8"
            )
            verify(crate, patch)
            # Publish the marker last, after all validation has succeeded.
            temporary = crate / f"{MARKER}.tmp"
            temporary.write_bytes(patch.read_bytes())
            temporary.replace(crate / MARKER)
        except BaseException:
            if crate.exists():
                shutil.rmtree(crate)
            raise
        print(f"winit {VERSION}: patch prepared.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--force", action="store_true", help="recreate even a verified crate")
    parser.add_argument("--check", action="store_true", help="only verify the prepared crate")
    args = parser.parse_args()
    if args.force and args.check:
        parser.error("--force and --check cannot be combined")
    try:
        prepare(Path(__file__).resolve().parents[1], args.force, args.check)
    except (PreparationError, OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"winit preparation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
