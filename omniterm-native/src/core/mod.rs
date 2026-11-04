// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Everything below the window: where data lives, saved connections, how sessions start,
//! settings and the workspace. Nothing here draws or blocks on the UI thread.

pub mod paths;
pub mod plan;
pub mod profile;
pub mod settings;
pub mod ssh_config;
pub mod text;
pub mod workspace;
pub mod x11;
