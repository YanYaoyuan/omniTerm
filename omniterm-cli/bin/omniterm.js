#!/usr/bin/env node
// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { main } from "../src/cli.js";

process.exitCode = await main(process.argv.slice(2));
