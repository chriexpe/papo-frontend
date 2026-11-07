#!/usr/bin/env python3
"""Check SDK dependency direction and reject platform types or UI error text.

Cargo metadata resolves renamed and transitive dependencies. Source checks add
coverage for UI types/macros and string-shaped error/status APIs; diagnostic
`thiserror` descriptions and protocol payload strings are not UI translations.
"""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

SDK_EDGES = {
    "papo-api": set(),
    "papo-sdk": {"papo-api"},
    "papo-sdk-ui": {"papo-sdk"},
    "papo-ffi": {"papo-sdk-ui"},
    "papo-test-server": {"papo-api"},
}
PRODUCTION = {"papo-api", "papo-sdk", "papo-sdk-ui", "papo-ffi"}
LEGACY = {"papo", "papo-core", "papo-android", "papo-desktop", "papo-call"}
PLATFORM = re.compile(r"^(?:gpui(?:[-_].*)?|egui(?:[-_].*)?|eframe|uniffi(?:[-_].*)?|jni|android-activity)$")
TYPE_REFERENCE = re.compile(r"\b(?:gpui(?:_\w+)?|egui(?:_\w+)?|eframe|uniffi(?:_\w+)?|androidx|compose)\s*::")
UI_TEXT = re.compile(r"\b(?:tr|gettext|ngettext|localize|translate)\s*[!(]")
STRING_ERROR = re.compile(
    r"\bResult\s*<[^;{}]*,\s*(?:String|&\s*'static\s+str)\s*>"
    r"|\b(?:error|status|user_message|display_message)\s*:\s*(?:Option\s*<\s*)?(?:String|&\s*'static\s+str)\b"
    r"|\bErr\s*\(\s*\""
)


def dependency_violations(metadata):
    """Return violations, resolving package names rather than dependency aliases."""
    packages = {p["id"]: p["name"] for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    violations = []
    for package_id, name in packages.items():
        if name not in SDK_EDGES:
            continue
        node = nodes[package_id]
        for dependency in node["deps"]:
            target = packages[dependency["pkg"]]
            test_backend = target == "papo-test-server" and all(
                kind["kind"] == "dev" for kind in dependency["dep_kinds"]
            )
            if target in SDK_EDGES and target not in SDK_EDGES[name] and not test_backend:
                violations.append(f"{name}: forbidden direct SDK dependency {target}")
        if name not in PRODUCTION:
            continue
        pending = [package_id]
        visited = set()
        while pending:
            current = pending.pop()
            if current in visited:
                continue
            visited.add(current)
            for dependency in nodes[current]["deps"]:
                dev_only = all(kind["kind"] == "dev" for kind in dependency["dep_kinds"])
                # SDK-owned test dependencies are checked; dependencies' own tests are not built.
                if current != package_id and dev_only:
                    continue
                target_id = dependency["pkg"]
                target = packages[target_id]
                if current == package_id and dev_only and target == "papo-test-server":
                    continue
                if target in LEGACY or target == "papo-test-server":
                    violations.append(f"{name}: forbidden production dependency {target}")
                if PLATFORM.fullmatch(target) and not (name == "papo-ffi" and target.startswith("uniffi")):
                    violations.append(f"{name}: forbidden platform dependency {target}")
                pending.append(target_id)
    return sorted(set(violations))


def rust_code(source, mask_literals=False):
    """Lex comments before literals in comment mode, retaining only live Rust code."""
    result = []
    cursor = 0
    while cursor < len(source):
        start = cursor
        if source.startswith("//", cursor):
            end = source.find("\n", cursor)
            cursor = len(source) if end < 0 else end
            result.append(" ")
            continue
        if source.startswith("/*", cursor):
            depth = 1
            cursor += 2
            while cursor < len(source) and depth:
                if source.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif source.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            result.append("\n" * source[start:cursor].count("\n") + " ")
            continue
        raw = re.match(r'(?:br|cr|r)(\#*)"', source[cursor:])
        if raw:
            terminator = '"' + raw.group(1)
            end = source.find(terminator, cursor + raw.end())
            cursor = len(source) if end < 0 else end + len(terminator)
            result.append('""' if mask_literals else source[start:cursor])
            continue
        if source[cursor] == '"':
            cursor += 1
            while cursor < len(source):
                if source[cursor] == "\\":
                    cursor += 2
                elif source[cursor] == '"':
                    cursor += 1
                    break
                else:
                    cursor += 1
            result.append('""' if mask_literals else source[start:cursor])
            continue
        # A lifetime has no closing quote; a char literal may contain a quote or slash.
        char = re.match(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\])'", source[cursor:])
        if char:
            cursor += char.end()
            result.append("''" if mask_literals else source[start:cursor])
            continue
        result.append(source[cursor])
        cursor += 1
    return "".join(result)


def without_comments(source):
    """Preserve literals while removing line/block comments, including nested blocks."""
    return rust_code(source)


def source_violations(source, package):
    """Reject UI references/copy and untyped error surfaces below client crates."""
    # Literals cannot refer to Rust types or invoke translation macros.
    tokens = rust_code(source, mask_literals=True)
    violations = []
    references = TYPE_REFERENCE.findall(tokens)
    if references and package != "papo-ffi":
        violations.append("platform type reference below bindings")
    elif package == "papo-ffi" and any(not reference.strip().startswith("uniffi") for reference in references):
        violations.append("platform widget type in bindings")
    if package in PRODUCTION and UI_TEXT.search(tokens):
        violations.append("UI translation belongs to clients")
    if package in {"papo-sdk", "papo-sdk-ui"} and STRING_ERROR.search(tokens):
        violations.append("errors/statuses must be typed, not UI strings")
    return violations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, help="read a Cargo metadata JSON snapshot")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        if args.metadata:
            metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
        else:
            output = subprocess.run(
                ["cargo", "metadata", "--format-version=1", "--all-features", "--locked"],
                cwd=root, check=True, encoding="utf-8", capture_output=True,
            )
            metadata = json.loads(output.stdout)
        violations = dependency_violations(metadata)
        for package in metadata["packages"]:
            if package["name"] not in SDK_EDGES:
                continue
            directory = Path(package["manifest_path"]).parent
            for source in sorted((directory / "src").rglob("*.rs")):
                for violation in source_violations(source.read_text(encoding="utf-8"), package["name"]):
                    violations.append(f"{source.relative_to(root)}: {violation}")
        if violations:
            print("\n".join(violations), file=sys.stderr)
            return 1
    except subprocess.CalledProcessError as error:
        print("SDK boundary check could not resolve Cargo metadata:", file=sys.stderr)
        print(error.stderr, file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError) as error:
        print(f"SDK boundary check could not complete: {error}", file=sys.stderr)
        return 1
    print("SDK dependency and source boundaries passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
