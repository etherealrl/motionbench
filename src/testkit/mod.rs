//! builders, fault injection and corpus checks shared by the tests. the `testkit` feature exposes
//! the builders to the replay fixture generator; everything else stays test-only

#[cfg(test)]
mod corpus;
#[cfg(test)]
pub mod faults;
#[cfg(test)]
pub mod fixtures;
pub mod traffic;
