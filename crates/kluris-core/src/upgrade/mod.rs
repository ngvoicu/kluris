//! What earlier releases left on a user's machine, found and removed: the
//! per-user sync services that ran `kluris sync --watch` ([`services`]) and
//! the agent skill copies in folders Kluris writes no skill to
//! ([`skills`]). This module goes at 4.0.

pub mod services;
pub mod skills;
