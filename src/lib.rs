//! Primordium as a library: the simulation, its config and the renderer.
//! The window (`main.rs`) and the headless lab harness (`bin/lab`) are both
//! built on it.

// Same reason as in main.rs: the `@veridikt` convention puts an annotation
// block after a `///` doc comment, which is exactly the shape this lint
// fires on.
#![allow(clippy::empty_line_after_doc_comments)]

// @veridikt
// kind: module
// name: Primordium
// purpose: "Library root exposing config, render and sim to the window binary and the lab harness"
// owner: "primordium-maintainers"
// because: "The lab harness used to live in an out-of-tree patch that added this file; it broke silently whenever a patched file changed, so the measurement tool now builds from the same tree as the sim it measures"

pub mod config;
pub mod render;
pub mod sim;
