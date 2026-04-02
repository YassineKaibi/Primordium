mod config;
mod render;
mod sim;

use config::WorldConfig;

fn main() {
    let config = WorldConfig::default();
    println!(
        "Primordium — {}x{} grid, seed {}",
        config.grid_width, config.grid_height, config.seed
    );
}
