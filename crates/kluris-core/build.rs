//! Rebuild when `vendored/` changes: the companions are embedded from it
//! (`include_dir!` tracks the files it saw, not files added or removed).

fn main() {
    println!("cargo:rerun-if-changed=../../vendored");
}
