//! Shared types, errors, and configuration for the AI Sandbox Engine.
//!
//! This crate contains no isolation logic—only pure data types that are
//! used across all other crates. Keeping it dependency-free avoids circular
//! dependencies.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod types;