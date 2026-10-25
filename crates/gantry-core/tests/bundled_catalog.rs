//! The bundled catalog in `data/questions/` parses and validates.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[test]
fn bundled_catalog_parses() {
    common::bundled();
}
