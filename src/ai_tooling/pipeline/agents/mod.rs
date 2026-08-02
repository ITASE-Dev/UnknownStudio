//! The three agents. Each owns its schema, its system prompt and the shape of
//! its payload, and knows nothing about the others — chaining is
//! [`LlmPipelineEngine`](super::LlmPipelineEngine)'s job.

pub mod deconstructor;
pub mod director;
pub mod prompt_engineer;
