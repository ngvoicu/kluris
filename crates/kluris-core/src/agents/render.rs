//! stub
pub const MARKER: &str = "";
pub struct SkillInput<'a> {
    pub brain_name: &'a str,
    pub brain_path: &'a str,
    pub has_git: bool,
    pub description: &'a str,
    pub companions: &'a [String],
    pub companion_home: &'a str,
}
pub fn skill_name(brain_name: &str) -> String { format!("kluris-{brain_name}") }
pub fn render_skill(_input: &SkillInput) -> String { String::new() }
pub fn posix_path(p: &str) -> String { p.to_string() }
pub fn is_generated_skill(_dir: &str, _text: &str) -> bool { false }
pub fn is_generated_workflow(_file: &str, _text: &str) -> bool { false }
