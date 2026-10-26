use std::io::Write;
use std::path::Path;

use super::options::Options;
use crate::ssh_ci::runner::Runner;

pub struct Ctx<'a> {
    pub runner: &'a mut dyn Runner,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub stop: &'a dyn Fn() -> bool,
    pub channel: String,
    pub pack: &'a mut dyn FnMut(&Path, &str) -> Result<String, String>,
    pub root: &'a Path,
}

pub fn linux(_options: &Options, _ctx: &mut Ctx) -> i32 {
    1
}
