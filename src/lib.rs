// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

pub mod auth;
mod cli;
pub mod config;
pub mod feedback;
pub mod instance;
pub mod launch_profile;
pub mod layout_migration;
mod migrate;
pub mod net;
pub mod storage;
mod time;
pub mod tui;

#[cfg(test)]
pub(crate) mod tests;

pub use cli::init as cli_init;
pub use migrate::run_legacy_rename as migrate_legacy_rename;
