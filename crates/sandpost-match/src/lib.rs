#![doc = include_str!("../README.md")]

mod candidates;
mod delta;
mod matcher;

pub use delta::MatchDelta;
pub use matcher::{MatchError, MatchResult, MatchStatistics, Matcher};
