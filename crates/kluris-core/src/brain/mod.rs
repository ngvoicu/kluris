//! The brain model: one walker shared by every command, the neuron index, the
//! path sandbox, glossary parsing, deprecation diagnostics and the
//! [`store::BrainStore`] read interface.

pub mod deprecation;
pub mod glossary;
pub mod neuron_index;
pub mod sandbox;
pub mod store;
pub mod walk;
