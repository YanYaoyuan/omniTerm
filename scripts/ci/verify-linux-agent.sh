#!/usr/bin/env bash
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
set -euo pipefail
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec "${PYTHON:-python3}" "$script_directory/verify-linux-agent.py" "$@"
