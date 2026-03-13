/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Execution engine for workflows.

pub(crate) mod executor;
pub mod rate_limiter;

pub use executor::{ExecutionMetadata, Executor, PauseRegistry};
pub use rate_limiter::{RateLimitConfig, RateLimiterRegistry};
