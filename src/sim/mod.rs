// Allow dead code during development: modules are tested but not yet
// wired into the binary's main loop.
#![allow(dead_code)]

pub mod actions;
pub mod cell;
pub mod diffusion;
pub mod energy;
pub mod genome;
pub mod phase;
pub mod spawner;
pub mod tick;
pub mod world;

// Simulation struct, public API: new(), step(), snapshot()
