#!/usr/bin/env python3
"""Prepare the checksum-pinned winit patch without building a helper CLI."""

import argparse
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.error
import urllib.request

VERSION = "0.30.13"
CHECKSUM = "a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d"
PATCH = Path(f"patches/winit+{VERSION}.patch")
# Outside target/: rust-cache cleans target/ during its post-job save.
DESTINATION = Path(f".papo/patch/winit-{VERSION}")
MARKER = ".papo-applied.patch"
ARCHIVE_URL = f"https://static.crates.io/crates/winit/winit-{VERSION}.crate"


class PreparationError(Exception):
    pass


def run_git(crate, patch, *flags):
    return subprocess.run(
        ["git", "-c", "core.autocrlf=false", "apply", *flags, str(patch)],
        cwd=crate, capture_output=True, text=True, check=False,
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
    directory = root / ".papo"
    directory.mkdir(exist_ok=True)
    with (directory / ".prepare-winit.lock").open("a+b") as lock:
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


def verify_archive_bytes(data):
    if hashlib.sha256(data).hexdigest() != CHECKSUM:
        raise PreparationError("The original winit archive has the wrong SHA-256.")
    return data


def archive_bytes(root):
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).expanduser()
    if not cargo_home.is_absolute():
        cargo_home = root / cargo_home
    archives = sorted((cargo_home / "registry/cache").glob(f"*/winit-{VERSION}.crate"))
    for archive in archives:
        return verify_archive_bytes(archive.read_bytes())
    # Cargo cannot fetch a missing crate through a manifest whose path
    # override has not been prepared. Bootstrap this single pinned archive.
    try:
        request = urllib.request.Request(ARCHIVE_URL, headers={"User-Agent": "Papo-winit-preparation"})
        with urllib.request.urlopen(request, timeout=45) as response:
            return verify_archive_bytes(response.read())
    except (urllib.error.URLError, TimeoutError) as error:
        raise PreparationError(f"Cannot download the pinned winit archive: {error}") from error


def unpack_archive(data, directory):
    prefix = f"winit-{VERSION}"
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        members = archive.getmembers()
        # Never trust archive paths, even with checksum pinning.
        for member in members:
            parts = PurePosixPath(member.name).parts
            if not parts or parts[0] != prefix or ".." in parts or not (
                member.isfile() or member.isdir()
            ):
                raise PreparationError(f"Invalid archive member: {member.name}")
        for member in members:
            target = directory.joinpath(*PurePosixPath(member.name).parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                source = archive.extractfile(member)
                if source is None:
                    raise PreparationError(f"Cannot read archive member: {member.name}")
                with source, target.open("wb") as output:
                    shutil.copyfileobj(source, output)
    return directory / prefix


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
        lockfile = root / "Cargo.lock"
        before = lockfile.read_bytes() if lockfile.exists() else None
        # Stage fully before replacing the path Cargo will resolve.
        with tempfile.TemporaryDirectory(prefix="winit-", dir=root / ".papo") as temporary:
            crate = unpack_archive(archive_bytes(root), Path(temporary))
            result = run_git(crate, patch, "--check")
            if result.returncode:
                raise PreparationError(f"Cannot apply winit patch:\n{result.stderr.strip()}")
            result = run_git(crate, patch)
            if result.returncode:
                raise PreparationError(f"Failed to apply winit patch:\n{result.stderr.strip()}")
            (crate / ".cargo-checksum.json").write_text(
                json.dumps({"package": CHECKSUM, "files": {}}), encoding="utf-8"
            )
            verify(crate, patch)
            (crate / MARKER).write_bytes(patch.read_bytes())
            if (lockfile.read_bytes() if lockfile.exists() else None) != before:
                raise PreparationError("Cargo.lock changed during preparation; stop concurrent Cargo commands.")
            destination = root / DESTINATION
            destination.parent.mkdir(parents=True, exist_ok=True)
            if destination.exists():
                shutil.rmtree(destination)
            crate.rename(destination)
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
    except (PreparationError, OSError, ValueError, KeyError, tarfile.TarError) as error:
        print(f"winit preparation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
