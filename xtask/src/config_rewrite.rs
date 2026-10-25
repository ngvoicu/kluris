//! `config-rewrite`: read a config with kluris-core and write it back, for
//! `parity/test_config_roundtrip.py` (no v3 command writes a config yet).

use std::path::Path;

use kluris_core::config::{
    read_brain_config, read_global_config, write_brain_config, write_global_config,
};

/// `config-rewrite global <config.yml>` or `config-rewrite brain <brain dir>`.
pub fn main(args: &[String]) -> Result<(), String> {
    match args {
        [kind, path] if kind == "global" => {
            let path = Path::new(path);
            let config = read_global_config(path).map_err(|e| e.to_string())?;
            write_global_config(path, &config).map_err(|e| e.to_string())
        }
        [kind, dir] if kind == "brain" => {
            let dir = Path::new(dir);
            let config = read_brain_config(dir).map_err(|e| e.to_string())?;
            write_brain_config(dir, &config).map_err(|e| e.to_string())
        }
        _ => Err("usage: config-rewrite global <config.yml> | brain <brain dir>".into()),
    }
}
