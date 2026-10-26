#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub parity: bool,
    pub parity_files: Vec<String>,
    pub cargo_args: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
}

pub fn parse(_args: &[String]) -> Result<Parsed, String> {
    Err(String::new())
}
