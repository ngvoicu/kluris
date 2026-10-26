//! Change sets: every write into a brain is one [`ChangeSet`], applied by the
//! store ([`super::store::BrainStore::apply`]).

/// How a [`FileWrite`] treats a file already at its path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WriteMode {
    /// Replace it atomically ([`crate::atomic::write_atomic`]): on any
    /// failure the old file stays byte-identical.
    #[default]
    Replace,
    /// Only create the file: one already there is an error and is left
    /// alone (a new brain's scaffold).
    CreateNew,
}

/// One file's new contents, by its `/`-separated brain-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    pub rel: String,
    pub contents: Vec<u8>,
    pub mode: WriteMode,
}

impl FileWrite {
    /// A write that replaces the file at `rel`.
    pub fn replace(rel: impl Into<String>, contents: impl Into<Vec<u8>>) -> FileWrite {
        FileWrite {
            rel: rel.into(),
            contents: contents.into(),
            mode: WriteMode::Replace,
        }
    }

    /// A write that creates `rel` and never overwrites a file there.
    pub fn create_new(rel: impl Into<String>, contents: impl Into<Vec<u8>>) -> FileWrite {
        FileWrite {
            rel: rel.into(),
            contents: contents.into(),
            mode: WriteMode::CreateNew,
        }
    }
}

/// A set of writes made together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// The revision the writes were computed against; `None` is the working
    /// tree as the store read it (dream; `commit` and `sync` will set it).
    pub base: Option<String>,
    /// Who made the change, and who approved it, when known.
    pub author: Option<String>,
    pub approver: Option<String>,
    /// In the order they are written; one entry per path.
    pub writes: Vec<FileWrite>,
}

/// A write that could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedWrite {
    pub rel: String,
    pub error: String,
}

/// What [`super::store::BrainStore::apply`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// The paths written, in the change set's order.
    pub written: Vec<String>,
    pub failed: Vec<FailedWrite>,
}
