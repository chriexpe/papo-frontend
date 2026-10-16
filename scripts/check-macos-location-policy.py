#!/usr/bin/env python3
"""Fail packaging/CI if app surfaces could request macOS location access."""
import plistlib
import re
import sys
from pathlib import Path


def check(keys):
    forbidden = [key for key in keys if key.startswith("NSLocation")]
    if forbidden:
        raise ValueError("App-surface isolation forbids location usage keys: " + ", ".join(forbidden))


if __name__ == "__main__":
    path = Path(sys.argv[1])
    if path.suffix == ".plist":
        check(plistlib.loads(path.read_bytes()))
    else:
        check(re.findall(r"<key>([^<]+)</key>", path.read_text()))
