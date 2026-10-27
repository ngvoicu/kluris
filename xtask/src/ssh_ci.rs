//! What `cargo xtask windows` and `cargo xtask linux` share: the programs a run
//! starts on this side ([`runner`]), the tarball of the worktree ([`pack`]),
//! its upload and the streamed run ([`session`]), what a run is ([`plan`]), how
//! it ends ([`outcome`]), the words of the command ([`words`]), and the
//! first-come queue for the machines that every runner on this Mac keeps,
//! ConsensFlow's too ([`queue`]). Each
//! machine's own scripts, refusals and stop stay with its task.

#[cfg(test)]
pub mod fake;
pub mod outcome;
pub mod pack;
pub mod plan;
pub mod queue;
pub mod runner;
pub mod session;
pub mod words;
