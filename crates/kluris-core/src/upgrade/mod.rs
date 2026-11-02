//! What earlier releases left behind, found and brought up to date: the
//! per-user sync services that ran `kluris sync --watch` ([`services`]),
//! the agent skill copies in folders Kluris writes no skill to
//! ([`skills`]), and a brain's README as an earlier release rendered it
//! ([`readme`]). This module goes at 4.0.

pub mod readme;
pub mod services;
pub mod skills;
