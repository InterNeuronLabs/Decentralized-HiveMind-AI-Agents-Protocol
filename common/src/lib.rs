// common/src/lib.rs
// Shared types and utilities for the cluster network.
// This crate has zero I/O — pure data types and computations only.

#![forbid(unsafe_code)]

pub mod credits;
pub mod identity;
pub mod tls;
pub mod types;
