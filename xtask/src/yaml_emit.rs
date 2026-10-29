//! `yaml-emit` and `yaml-load`: the Rust side of the YAML differential test
//! (`parity/test_yaml_differential.py`), one JSON line in, one JSON line out.

use std::io::{self, BufRead, Write};

use kluris_core::yaml::tagged_json::{decode, encode};
use kluris_core::yaml::{self, Profile};
use serde_json::{Value as Json, json};

pub fn emit(args: &[String]) -> Result<(), String> {
    let profile = match args {
        [flag, profile] if flag == "--profile" => match profile.as_str() {
            "md" => Profile::Md,
            "yml" => Profile::Yml,
            "config" => Profile::Config,
            other => return Err(format!("unknown profile '{other}'")),
        },
        _ => return Err("yaml-emit needs --profile md|yml|config".into()),
    };
    each_line(|request| {
        let Ok(map) = decode(&request)?.into_map() else {
            return Err("yaml-emit writes mappings only".into());
        };
        Ok(Json::String(yaml::emit(&map, profile)))
    })
}

pub fn load(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("yaml-load takes no arguments".into());
    }
    each_line(|request| {
        let text = request.as_str().ok_or("yaml-load reads JSON strings")?;
        Ok(match yaml::load(text) {
            Ok(value) => json!({ "value": encode(&value) }),
            Err(error) => json!({ "error": error.to_string() }),
        })
    })
}

/// Answer each stdin line, flushing every answer: the caller waits for it.
fn each_line(answer: impl Fn(Json) -> Result<Json, String>) -> Result<(), String> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| format!("cannot read stdin: {e}"))?;
        let request: Json =
            serde_json::from_str(&line).map_err(|e| format!("bad JSON line: {e}"))?;
        let reply = answer(request)?;
        writeln!(out, "{reply}")
            .and_then(|()| out.flush())
            .map_err(|e| format!("cannot write stdout: {e}"))?;
    }
    Ok(())
}
