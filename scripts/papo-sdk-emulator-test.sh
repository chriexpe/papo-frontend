#!/usr/bin/env bash
# Require real instrumentation results; an empty test discovery is not a pass.
set -euo pipefail
cd "$(dirname "$0")/.."

adb install -r android/sdk-proof/build/outputs/apk/debug/sdk-proof-debug.apk
adb install -r android/sdk-proof/build/outputs/apk/androidTest/debug/sdk-proof-debug-androidTest.apk
report=android/sdk-proof/build/reports/instrumentation.txt
mkdir -p android/sdk-proof/build/reports
adb shell am instrument -w -r \
    -e class io.github.chriexpe.papo.sdkproof.TimelineProofTest \
    io.github.chriexpe.papo.sdkproof.test/androidx.test.runner.AndroidJUnitRunner | tee "$report"
python3 - "$report" <<'PY'
from pathlib import Path
import re
import sys

result = Path(sys.argv[1]).read_text()
passed = re.search(r"OK \((\d+) tests?\)", result)
if not passed or int(passed[1]) != 2 or "FAILURES" in result or "INSTRUMENTATION_FAILED" in result:
    raise SystemExit("SDK instrumentation did not pass both proof tests")
PY
