//! Bakes the target triple into the binary (`KLURIS_TARGET`): `self-update`
//! picks the release archive built for the same target, so a Linux machine
//! that got the static musl build keeps updating to musl.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=KLURIS_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
