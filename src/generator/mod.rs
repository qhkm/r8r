//! Prompt-to-workflow generation.
//!
//! Generate r8r workflow YAML from natural language prompts using LLMs.

pub mod context;
pub mod prompt;

pub use context::GeneratorContext;
