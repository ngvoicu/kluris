pub const HOST: &str = "";
pub const TASK_LIMIT: u32 = 1024;
pub const RUN_FILE: &str = ".kluris-run.sh";
pub const UV_SHA256: &str = "";
pub const UV_URL: &str = "";

#[derive(Debug, Clone)]
pub struct Layout {
    pub poker_lab: String,
    pub windows_home: String,
    pub windows_owner: String,
}

impl Layout {
    pub fn zeelin() -> Self {
        Self {
            poker_lab: String::new(),
            windows_home: String::new(),
            windows_owner: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Spec<'a> {
    pub id: &'a str,
    pub channel: &'a str,
    pub jobs: u32,
    pub tasks: u32,
    pub parity: bool,
    pub parity_files: &'a [String],
    pub cargo_args: &'a [String],
}

pub fn probe(_layout: &Layout) -> String {
    String::new()
}
pub fn bootstrap(_id: &str, _layout: &Layout) -> String {
    String::new()
}
pub fn run_script(_spec: &Spec) -> String {
    String::new()
}
pub fn stop_script(_id: &str) -> String {
    String::new()
}
pub fn cleanup_script(_id: &str) -> String {
    String::new()
}
pub fn probe_line(_layout: &Layout) -> String {
    String::new()
}
pub fn bootstrap_line(_id: &str, _layout: &Layout) -> String {
    String::new()
}
pub fn stop_line(_id: &str) -> String {
    String::new()
}
pub fn cleanup_line(_id: &str) -> String {
    String::new()
}
pub fn quote(_text: &str) -> String {
    String::new()
}
pub fn remote_tarball(_id: &str) -> String {
    String::new()
}
