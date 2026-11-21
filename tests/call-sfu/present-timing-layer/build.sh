#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
cc -O2 -shared -fPIC -Wall -Wextra -Werror \
    -o libVkLayer_papo_present_timing.so \
    papo_present_timing_layer.c \
    -lpthread
