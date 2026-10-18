//! Native API contract and Stage 0 service. No storage operations are implemented.
pub mod api_doc;
pub mod app;
pub mod config;
pub mod dto;
pub mod error;
pub mod extract;
pub mod health;
pub mod middleware;
pub mod v1;

pub mod control;
pub mod receipt;

pub mod bootstrap;
