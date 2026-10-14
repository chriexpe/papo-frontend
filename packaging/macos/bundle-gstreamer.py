#!/usr/bin/env python3
"""Relocate Homebrew's GStreamer runtime into Papo.app on a macOS build host.

Unlike Windows DLL lookup, Mach-O load commands carry install names. Copying
.dylibs is insufficient: every reference must be rewritten and checked.
Fail closed if any non-system binary is unresolved or points outside the app.
"""
from __future__ import annotations

import argparse
from collections import deque
from pathlib import Path
import re
import shutil
import subprocess
import sys


def run(*args: str) -> str:
    result = subprocess.run(args, check=True, text=True, capture_output=True)
    return result.stdout


def brew_prefix(formula: str | None = None) -> Path:
    return Path(run("brew", "--prefix", *([formula] if formula else [])).strip())


def dependencies(binary: Path) -> list[str]:
    lines = run("otool", "-L", str(binary)).splitlines()[1:]
    result: list[str] = []
    for line in lines:
        match = re.match(r"^\s*(\S+)\s+\(", line)
        if match:
            result.append(match.group(1))
    return result


def rpaths(binary: Path) -> list[str]:
    lines = run("otool", "-l", str(binary)).splitlines()
    paths: list[str] = []
    looking = False
    for line in lines:
        token = line.strip()
        if token == "cmd LC_RPATH":
            looking = True
        elif looking and token.startswith("path "):
            paths.append(token[5:].split(" (offset ", 1)[0])
            looking = False
        elif token.startswith("cmd "):
            looking = False
    return paths


def system_dependency(name: str) -> bool:
    return (
        name.startswith("/usr/lib/")
        or name.startswith("/System/Library/")
        or name.startswith("/Library/Apple/System/")
    )


class Bundle:
    def __init__(self, app: Path, prefix: Path):
        self.app = app.resolve()
        self.prefix = prefix.resolve()
        self.frameworks = self.app / "Contents/Frameworks"
        self.plugins = self.app / "Contents/PlugIns/gstreamer-1.0"
        self.gio = self.app / "Contents/PlugIns/gio-modules"
        self.helpers = self.app / "Contents/Helpers"
        self.exe = self.app / "Contents/MacOS/papo"
        self.queue: deque[tuple[Path, Path]] = deque()
        self.seen: set[Path] = set()
        self.copied: dict[Path, Path] = {}
        self.names: dict[str, Path] = {}
        self.changed: set[Path] = set()
        for path in (self.frameworks, self.plugins, self.gio, self.helpers):
            path.mkdir(parents=True, exist_ok=True)

    def within_brew(self, path: Path) -> bool:
        return path.is_relative_to(self.prefix)

    def enqueue_library(self, path: Path, preferred: str) -> Path:
        resolved = path.resolve(strict=True)
        if not self.within_brew(resolved):
            raise RuntimeError(f"unexpected external dependency: {path} -> {resolved}")
        if resolved in self.copied:
            return self.copied[resolved]
        basename = Path(preferred).name
        if basename in self.names and self.names[basename] != resolved:
            raise RuntimeError(f"conflicting library install name: {basename}")
        self.names[basename] = resolved
        destination = self.frameworks / basename
        shutil.copy2(resolved, destination)
        destination.chmod(destination.stat().st_mode | 0o200)
        self.copied[resolved] = destination
        self.queue.append((resolved, destination))
        return destination

    def stage_plugin(self, source: Path, destination_dir: Path) -> None:
        destination = destination_dir / source.name
        if destination.exists():
            raise RuntimeError(f"duplicate plugin: {destination}")
        shutil.copy2(source.resolve(strict=True), destination)
        destination.chmod(destination.stat().st_mode | 0o200)
        self.queue.append((source, destination))

    def stage_runtime(self) -> None:
        gst_prefix = brew_prefix("gstreamer")
        nice_prefix = brew_prefix("libnice-gstreamer")
        plugin_dirs = [
            gst_prefix / "lib/gstreamer-1.0",
            nice_prefix / "lib/gstreamer-1.0",
        ]
        count = 0
        for plugin_dir in plugin_dirs:
            if not plugin_dir.is_dir():
                raise RuntimeError(f"GStreamer plugin directory missing: {plugin_dir}")
            for plugin in sorted(plugin_dir.iterdir()):
                if plugin.is_file() and plugin.suffix in (".dylib", ".so"):
                    self.stage_plugin(plugin, self.plugins)
                    count += 1
        if not count:
            raise RuntimeError("no GStreamer plugins found")
        scanner_candidates = [
            gst_prefix / "libexec/gstreamer-1.0/gst-plugin-scanner",
            self.prefix / "libexec/gstreamer-1.0/gst-plugin-scanner",
            gst_prefix / "lib/gstreamer-1.0/gst-plugin-scanner",
        ]
        scanner = next((p for p in scanner_candidates if p.is_file()), None)
        if scanner is None:
            raise RuntimeError("GStreamer plugin scanner not found")
        self.stage_plugin(scanner, self.helpers)

        gio_modules = self.prefix / "lib/gio/modules"
        if gio_modules.is_dir():
            for plugin in sorted(gio_modules.iterdir()):
                if plugin.is_file() and plugin.suffix in (".dylib", ".so"):
                    self.stage_plugin(plugin, self.gio)

        self.queue.append((self.exe, self.exe))
        print(f"Staged {count} GStreamer plugins, scanner and optional GIO modules")

    def resolve(self, dependency: str, original: Path) -> Path:
        if dependency.startswith("/"):
            candidate = Path(dependency)
            if candidate.exists():
                return candidate
        elif dependency.startswith("@loader_path/"):
            candidate = original.parent / dependency[len("@loader_path/"):]
            if candidate.exists():
                return candidate
        elif dependency.startswith("@executable_path/"):
            candidate = self.exe.parent / dependency[len("@executable_path/"):]
            if candidate.exists():
                return candidate
        elif dependency.startswith("@rpath/"):
            suffix = dependency[len("@rpath/"):]
            for entry in rpaths(original):
                entry = entry.replace("@loader_path", str(original.parent))
                entry = entry.replace("@executable_path", str(self.exe.parent))
                candidate = Path(entry) / suffix
                if candidate.exists():
                    return candidate
            for candidate in (
                original.parent / suffix,
                self.prefix / "lib" / suffix,
                self.prefix / "opt/gstreamer/lib" / suffix,
            ):
                if candidate.exists():
                    return candidate

        raise RuntimeError(f"cannot resolve {dependency} referenced by {original}")

    def process(self) -> None:
        self.stage_runtime()
        while self.queue:
            original, destination = self.queue.popleft()
            if destination in self.seen:
                continue
            self.seen.add(destination)
            replacements: list[tuple[str, str]] = []
            for name in dependencies(original):
                if system_dependency(name):
                    continue
                if name.startswith("@rpath/libswift") or name.startswith("@rpath/libclang_rt"):
                    # Apple-supplied runtime resolved through the OS toolchain.
                    continue
                # Ignore Mach-O's own install ID; reset it below for dylibs.
                if original != self.exe and name == str(original):
                    continue
                source = self.resolve(name, original)
                if system_dependency(str(source)):
                    continue
                if not self.within_brew(source.resolve()):
                    raise RuntimeError(f"unexpected non-system dylib: {name} => {source}")
                staged = self.enqueue_library(source, Path(name).name)
                replacements.append((name, "@rpath/" + staged.name))

            for old, new in replacements:
                if old != new:
                    run("install_name_tool", "-change", old, new, str(destination))

            # Rewrite the identity of each copied dylib as well as its users.
            if destination.parent == self.frameworks:
                run("install_name_tool", "-id", "@rpath/" + destination.name, str(destination))

            for value in rpaths(destination):
                if value.startswith(str(self.prefix)):
                    run("install_name_tool", "-delete_rpath", value, str(destination))

            if destination.parent == self.frameworks:
                load_path = "@loader_path"
            elif destination.parent == self.plugins:
                load_path = "@loader_path/../../Frameworks"
            elif destination.parent == self.gio:
                load_path = "@loader_path/../../Frameworks"
            else:
                load_path = "@executable_path/../Frameworks" if destination == self.exe \
                    else "@loader_path/../Frameworks"

            if load_path not in rpaths(destination):
                run("install_name_tool", "-add_rpath", load_path, str(destination))
            self.changed.add(destination)

        if not self.copied:
            raise RuntimeError("runtime has no bundled native libraries")
        for binary in sorted(self.changed):
            for dependency in dependencies(binary):
                if system_dependency(dependency):
                    continue
                if dependency.startswith("@rpath/"):
                    path = self.frameworks / Path(dependency).name
                    if path.exists():
                        continue
                if dependency.startswith("@rpath/libswift") or dependency.startswith("@rpath/libclang_rt"):
                    continue
                raise RuntimeError(f"unrelocated dependency in {binary}: {dependency}")
            if any(path.startswith(str(self.prefix)) for path in rpaths(binary)):
                raise RuntimeError(f"Homebrew LC_RPATH remains in {binary}")

        for binary in sorted(self.changed):
            # install_name_tool invalidates existing vendor signatures.
            run("codesign", "--force", "--sign", "-", str(binary))

        print(f"Bundled {len(self.copied)} dependent libraries and "
              f"{len(self.changed)} Mach-O images into {self.app}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("app", type=Path)
    args = parser.parse_args()
    app = args.app.resolve()
    if not (app / "Contents/MacOS/papo").is_file():
        parser.error(f"not a Papo app: {app}")
    Bundle(app, brew_prefix()).process()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"GStreamer bundle error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
