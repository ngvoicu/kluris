//! `sync-specmint` (red-phase stubs).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

pub const NAMES: [&str; 4] = [
    "specmint-core",
    "specmint-tdd",
    "specmint-core-html",
    "specmint-tdd-html",
];

pub type Tree = BTreeMap<String, Vec<u8>>;

pub fn compat_section(_name: &str) -> String {
    String::new()
}
pub fn sanitize_for_kluris(_name: &str, c: &str) -> Result<String, String> {
    Ok(c.to_string())
}
pub fn forbidden_hits(_c: &str) -> Vec<&'static str> {
    Vec::new()
}
pub fn path_mentions(_t: &str) -> Vec<String> {
    Vec::new()
}
pub fn missing_mentions(_t: &Tree) -> Result<Vec<String>, String> {
    Ok(Vec::new())
}
pub fn build(_n: &str, _s: &Path, _o: &mut impl Write) -> Result<Tree, String> {
    Err("stub".into())
}
pub fn read_tree(_r: &Path) -> Tree {
    Tree::new()
}
pub fn line_count(_d: &[u8]) -> usize {
    0
}
pub fn report(_n: &str, _o: &Tree, _w: &Tree, _f: bool, _out: &mut impl Write) -> bool {
    false
}
pub fn run(_a: &[String], _root: &Path, _o: &mut impl Write) -> Result<i32, String> {
    Err("stub".into())
}

#[cfg(test)]
mod tests;
