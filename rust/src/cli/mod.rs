//! The command-line surface, as opposed to the MCP server.
//!
//! `main` stays a thin dispatcher; each subcommand lives with the others it resembles.
//! [`tools`] prints the catalogue, [`doctor`] answers whether this machine will work,
//! [`report`] holds the parts doctor reports, and [`subcommands`] carries the rest.

pub mod doctor;
pub mod report;
pub mod subcommands;
pub mod tools;
