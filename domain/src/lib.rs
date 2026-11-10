//! Questions, configuration and verdict math, shared by every other crate.

pub mod config;
pub mod question;
pub mod verdict;

/// One probability per label of the question, in `Kind::labels()` order, summing to 1.
pub type Probs = Vec<f64>;
