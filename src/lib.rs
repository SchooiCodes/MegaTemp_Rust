//! MegaTemp core library.
//!
//! Everything the terminal UI and the GUI need lives here: the MEGA client,
//! mail providers, browser automation, credential storage, configuration and
//! the shared rendering helpers. The single `megatemp` binary is a thin shell
//! over this crate: it launches the GUI by default and runs the CLI with flags.

#![allow(dead_code)]

pub mod browser;
pub mod cli;
pub mod config;
pub mod console;
pub mod credentials;
pub mod crypto;
pub mod disk;
pub mod gui;
pub mod hashcash;
pub mod lock;
pub mod mail;
pub mod mega;
pub mod menu;
pub mod models;
pub mod password;
pub mod proxy;
pub mod services;
pub mod update;
pub mod util;
