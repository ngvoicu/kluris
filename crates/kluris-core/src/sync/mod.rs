//! Git auto-sync (SPEC Phase 7): `commit` first; `sync` and `resolve` build
//! on its change-set model. `fresh` brings a brain up to date before
//! `wake-up` and `search` read it.

pub mod commit;
pub mod fresh;
