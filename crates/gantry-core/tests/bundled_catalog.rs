//! The bundled catalog in `data/questions/` parses and validates.

mod common;

#[test]
fn bundled_catalog_parses() {
    common::bundled();
}
