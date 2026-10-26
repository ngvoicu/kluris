//! Documents at the deepest nesting the reader accepts load, compare, clone,
//! write and drop on a 1 MiB stack (Windows' main-thread default), in a debug
//! build. A stack overflow aborts the process, so these live in their own
//! test binary.

use std::thread;

use kluris_core::yaml::{Map, Profile, Value, emit, load};

/// Below the composer's limit of 490 levels.
const DEPTH: usize = 480;

const ONE_MIB: usize = 1 << 20;

fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    thread::Builder::new()
        .stack_size(ONE_MIB)
        .spawn(f)
        .expect("spawn a thread")
        .join()
        .expect("the thread finished")
}

fn deep_flow_sequence() -> String {
    "[".repeat(DEPTH) + &"]".repeat(DEPTH)
}

fn deep_flow_mapping() -> String {
    "{a: ".repeat(DEPTH) + "x" + &"}".repeat(DEPTH)
}

fn deep_block_mapping() -> String {
    (0..DEPTH)
        .map(|i| format!("{}a:\n", "  ".repeat(i)))
        .collect::<String>()
        + &"  ".repeat(DEPTH)
        + "x\n"
}

/// The values built without the reader, so a test of one step does not
/// depend on another.
fn nested_seq() -> Value {
    let mut value = Value::Seq(vec![]);
    for _ in 1..DEPTH {
        value = Value::Seq(vec![value]);
    }
    value
}

fn nested_map() -> Value {
    let mut value = Value::Str("x".into());
    for _ in 0..DEPTH {
        let mut map = Map::new();
        map.insert("a".into(), value);
        value = Value::Map(map);
    }
    value
}

fn depth_of(value: &Value) -> usize {
    let mut depth = 0;
    let mut current = value;
    loop {
        match current {
            Value::Seq(items) if !items.is_empty() => current = &items[0],
            Value::Map(map) if !map.is_empty() => current = &map[0],
            Value::Seq(_) | Value::Map(_) => return depth + 1,
            _ => return depth,
        }
        depth += 1;
    }
}

#[test]
fn deep_flow_sequence_loads_on_a_1_mib_stack() {
    let depth = on_small_stack(|| load(&deep_flow_sequence()).map(|v| depth_of(&v)));
    assert_eq!(depth, Ok(DEPTH));
}

#[test]
fn deep_flow_mapping_loads_on_a_1_mib_stack() {
    let depth = on_small_stack(|| load(&deep_flow_mapping()).map(|v| depth_of(&v)));
    assert_eq!(depth, Ok(DEPTH));
}

#[test]
fn deep_block_mapping_loads_on_a_1_mib_stack() {
    let depth = on_small_stack(|| load(&deep_block_mapping()).map(|v| depth_of(&v)));
    assert_eq!(depth, Ok(DEPTH));
}

#[test]
fn too_deep_is_an_error_on_a_1_mib_stack() {
    let text = "[".repeat(5000) + &"]".repeat(5000);
    assert!(on_small_stack(move || load(&text).is_err()));
}

#[test]
fn deep_values_compare_and_clone_on_a_1_mib_stack() {
    assert!(on_small_stack(|| {
        let (a, b) = (nested_seq(), nested_seq());
        let c = a.clone();
        let (m, n) = (nested_map(), nested_map());
        a == b && c == a && m == n.clone() && a != m
    }));
}

#[test]
fn deep_values_drop_on_a_1_mib_stack() {
    on_small_stack(|| {
        drop(nested_seq());
        drop(nested_map());
    });
}

#[test]
fn deep_values_are_written_on_a_1_mib_stack() {
    let written = on_small_stack(|| {
        let mut map = Map::new();
        map.insert("seq".into(), nested_seq());
        map.insert("map".into(), nested_map());
        [Profile::Md, Profile::Yml, Profile::Config].map(|profile| emit(&map, profile).len())
    });
    assert!(written.iter().all(|&len| len > DEPTH));
}
