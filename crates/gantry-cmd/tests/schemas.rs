//! The `--json` output schemas under `docs/schema/` must match the types.
//! Regenerate with `GANTRY_UPDATE_SCHEMAS=1 cargo test -p gantry-cmd --test schemas`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

#[test]
fn published_schemas_are_current() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schema");
    let update = std::env::var_os("GANTRY_UPDATE_SCHEMAS").is_some();
    let mut stale = Vec::new();
    for (name, schema) in gantry_cmd::schema::all() {
        let path = dir.join(format!("{name}.json"));
        let want = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &want).unwrap();
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(want.as_str()) {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "stale schemas {stale:?}; run GANTRY_UPDATE_SCHEMAS=1 cargo test -p gantry-cmd --test schemas"
    );
}
