/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! n8n workflow migration.

pub mod converter;
pub mod expressions;
pub mod node_map;
pub mod parser;

pub use converter::convert_n8n_workflow;
