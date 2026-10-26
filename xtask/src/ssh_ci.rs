//! What `cargo xtask windows` and `cargo xtask linux` share: the programs a run
//! starts on this side ([`runner`]), the tarball of the worktree ([`pack`]),
//! what a run is ([`plan`]), how it ends ([`outcome`]) and the words of
//! the command ([`words`]). Each machine's own
//! scripts, refusals and stop stay with its task.

#[cfg(test)]
pub mod fake;
pub mod outcome;
pub mod pack;
pub mod plan;
pub mod runner;
pub mod words;
