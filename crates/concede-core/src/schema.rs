//! JSON Schemas for the record files, generated from the Rust types (ADR 0005).
//!
//! The generated files are checked in under `docs/schema/` so other tools can validate
//! `.concede/` without Concede. A test fails when they drift from the types; regenerate
//! with `CONCEDE_UPDATE_SCHEMA=1 cargo test -p concede-core schema`.

use schemars::{JsonSchema, schema_for};

use crate::model::{Capture, Decision, Event};

/// `(file name, pretty-printed schema)` for every record kind.
pub fn all() -> Vec<(&'static str, String)> {
    vec![
        ("decision.schema.json", render::<Decision>()),
        ("event.schema.json", render::<Event>()),
        ("capture.schema.json", render::<Capture>()),
    ]
}

fn render<T: JsonSchema>() -> String {
    let schema = sorted(schema_for!(T).to_value());
    // Serialising a JSON value cannot fail.
    let mut text = serde_json::to_string_pretty(&schema).unwrap_or_default();
    text.push('\n');
    text
}

/// Rebuild a JSON value with every object's keys in sorted order.
///
/// Whether `serde_json` keeps insertion order or sorts keys depends on Cargo feature
/// unification: other crates in the build (GPUI) enable `preserve_order`. Sorting
/// explicitly makes the output identical however the crate is built.
fn sorted(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn checked_in_schemas_match_the_types() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/schema");
        let update = std::env::var_os("CONCEDE_UPDATE_SCHEMA").is_some();
        let mut stale = Vec::new();
        for (name, generated) in super::all() {
            let path = dir.join(name);
            if update {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&path, &generated).unwrap();
            } else {
                // Ignore line endings: a Windows checkout may have converted LF to CRLF.
                let on_disk = std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .replace("\r\n", "\n");
                if on_disk != generated {
                    stale.push(name);
                }
            }
        }
        assert!(
            stale.is_empty(),
            "docs/schema is out of date for {stale:?}; run `CONCEDE_UPDATE_SCHEMA=1 cargo test -p concede-core schema`"
        );
    }
}
