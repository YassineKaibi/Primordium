//! One-shot modes that print a table and exit: the archetype economy, colour
//! separation, rendered frames and niche sizes.

use serde_json::json;

use primordium::config::WorldConfig;
use primordium::render::Renderer;
use primordium::render::color::{ColorMode, cell_to_rgba};
use primordium::sim::Simulation;
use primordium::sim::actions::{mapped_maturity_age, mapped_reproduction_cooldown};
use primordium::sim::cell::Cell;
use primordium::sim::energy;
use primordium::sim::genome::{self, Genome};
use primordium::sim::phase::apply_phase_modifiers;
use primordium::sim::spawner::{ARCHETYPES, Archetype, archetype_genome, random_genome};
use primordium::sim::world::World;

use crate::cli::r3;

/// Income against upkeep for the four archetypes `preset_archetypes` seeds,
/// each in its own niche. Use it before and after any economy change.
pub fn archetypes(config: &WorldConfig) {
    let prey = archetype_genome(Archetype::Photosynthesizer).decode(config);
    let prey_armor = prey.get(genome::ARMOR) * 255.0;
    for kind in ARCHETYPES {
        let g = archetype_genome(kind);
        let d = g.decode(config);
        let cost = energy::metabolic_cost(&d, 128, config);
        let cap = energy::storage_cap(&d, config);
        let photo_full = energy::photo_income(d.get(genome::PHOTOSYNTHESIS_RATE), 255, config);
        let thermo_40 = energy::thermo_income(
            d.get(genome::THERMOSYNTHESIS_RATE),
            config.vent_output,
            40,
            0,
            config.vent_cycle,
        );
        let (scav_corpse, _) = energy::scavenge_income(
            d.get(genome::SCAVENGE_ABILITY),
            config.corpse_biomass,
            config,
        );
        let mut hungry = d.clone();
        apply_phase_modifiers(&mut hungry, &g, 1);
        let sated_bite = (d.get(genome::ATTACK_POWER) * 255.0 - prey_armor).max(0.0);
        let hungry_bite = (hungry.get(genome::ATTACK_POWER) * 255.0 - prey_armor).max(0.0);
        println!(
            "{}",
            json!({
                "archetype": kind.name(), "upkeep": r3(cost as f64), "cap": r3(cap as f64),
                "speed": r3(d.get(genome::SPEED) as f64),
                "income": {
                    "photo_full_sun": r3(photo_full as f64),
                    "thermo_40_per_vent": r3(thermo_40 as f64),
                    "scav_one_corpse_tick": r3(scav_corpse as f64),
                },
                "maturity_ticks": mapped_maturity_age(&d, config),
                "cooldown_ticks": mapped_reproduction_cooldown(&d),
                "armor": (d.get(genome::ARMOR) * 255.0).round(),
                "bite_vs_prey": { "sated": sated_bite.round(), "hungry": hungry_bite.round() },
                "runway_ticks": (cap / cost).round(),
            })
        );
    }
}

fn hue_of(rgba: [u8; 4]) -> f32 {
    let (r, g, b) = (
        rgba[0] as f32 / 255.0,
        rgba[1] as f32 / 255.0,
        rgba[2] as f32 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d < 1e-6 {
        return 0.0;
    }
    let h = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    h.rem_euclid(360.0)
}

fn hue_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    d.min(360.0 - d)
}

/// How far colour moves for a one-byte mutation, against two unrelated
/// genomes. The old hash mapping scored 87.6 vs 90.0 degrees.
pub fn color_check(config: &WorldConfig) {
    use rand::{Rng, SeedableRng};

    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let mut world = World::new(config);
    let (mut sib_sum, mut unrel_sum, mut same_band) = (0.0f64, 0.0f64, 0u32);
    let n = 2000;

    let hue_for = |world: &mut World, g: &Genome| -> f32 {
        let id = world.spawn_cell(Cell::new(g.clone(), 50.0, (0, 0)));
        let idx = world.tile_index(0, 0);
        world.current_grid_mut()[idx].cell_id = id;
        let snap = world.snapshot(config);
        let h = hue_of(cell_to_rgba(&snap.cells[0], ColorMode::Genetic));
        world.current_grid_mut()[idx].cell_id = 0;
        world.kill_cell(id);
        h
    };

    for _ in 0..n {
        let parent = random_genome(&mut rng, config);
        let mut sibling = parent.clone();
        let i = rng.gen_range(0..genome::GENOME_LEN);
        sibling.data[i] = sibling.data[i].wrapping_add(if rng.r#gen::<bool>() { 1 } else { 255 });
        let stranger = random_genome(&mut rng, config);
        let hp = hue_for(&mut world, &parent);
        let hs = hue_for(&mut world, &sibling);
        let hu = hue_for(&mut world, &stranger);
        sib_sum += hue_dist(hp, hs) as f64;
        unrel_sum += hue_dist(hp, hu) as f64;
        if hue_dist(hp, hs) < 45.0 {
            same_band += 1;
        }
    }
    println!(
        "{}",
        json!({ "color_check": {
            "mean_hue_shift_one_byte": r3(sib_sum / n as f64),
            "mean_hue_shift_unrelated": r3(unrel_sum / n as f64),
            "siblings_in_same_band_pct": r3(same_band as f64 * 100.0 / n as f64),
        }})
    );
}

/// FNV-1a over `bytes`, folded into `h`.
fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
}

/// A hash of the whole world state: every live cell (id and every field,
/// genome included), its record, and every field of every tile.
fn state_hash(sim: &Simulation) -> u64 {
    let w = sim.world();
    let mut h = 0xcbf29ce484222325u64;
    fnv(&mut h, &w.tick.to_le_bytes());
    for (id, c) in w.live() {
        fnv(&mut h, &id.to_le_bytes());
        fnv(&mut h, &c.genome.data);
        for f in [c.energy, c.temp_acclimation] {
            fnv(&mut h, &f.to_bits().to_le_bytes());
        }
        fnv(&mut h, &c.age.to_le_bytes());
        fnv(&mut h, &c.position.0.to_le_bytes());
        fnv(&mut h, &c.position.1.to_le_bytes());
        fnv(&mut h, &c.cooldown_remaining.to_le_bytes());
        fnv(&mut h, &c.last_damage_tick.to_le_bytes());
        fnv(
            &mut h,
            &[
                c.active_phase,
                c.phase_ticks,
                c.venom_ticks,
                c.venom_damage,
                c.memory_dir.0 as u8,
                c.memory_dir.1 as u8,
                c.memory_ticks,
            ],
        );
        let r = w.record(id);
        fnv(&mut h, &r.lineage.to_le_bytes());
        for f in r.income.iter().chain(std::iter::once(&r.upkeep)) {
            fnv(&mut h, &f.to_bits().to_le_bytes());
        }
    }
    for t in w.current_grid() {
        fnv(&mut h, &t.cell_id.to_le_bytes());
        for f in [t.decay_energy, t.decay_fade, t.pheromone, t.toxin] {
            fnv(&mut h, &f.to_bits().to_le_bytes());
        }
        fnv(&mut h, &[t.temperature, t.sunlight]);
    }
    h
}

/// Print a full-state hash every `every` ticks, per seed. Two trees (or two
/// configs) that print the same lines ran bit-identically: this is the check
/// that a knob left at its default changes nothing.
pub fn hash(config: &WorldConfig, seeds: Vec<u64>, ticks: u64, every: u64, jobs: usize) {
    let lines = crate::cli::run_parallel(seeds, jobs, |&seed| {
        let mut sim = Simulation::new(WorldConfig {
            seed,
            ..config.clone()
        });
        let mut out = Vec::new();
        for t in 0..=ticks {
            if t > 0 {
                sim.step();
            }
            if t % every == 0 || t == ticks {
                out.push(json!({
                    "seed": seed, "tick": t, "pop": sim.world().population(),
                    "hash": format!("{:016x}", state_hash(&sim)),
                }));
            }
        }
        out
    });
    for line in lines.into_iter().flatten() {
        println!("{line}");
    }
}

/// Write what the window would show to PPM files, for every colour mode.
pub fn render_frames(config: &WorldConfig, ticks: u64, shots: &[u64], prefix: &str) {
    let mut sim = Simulation::new(config.clone());
    let mut renderer = Renderer::new(config.grid_width, config.grid_height);
    let (w, h) = (config.grid_width as usize, config.grid_height as usize);
    for t in 0..=ticks {
        if t > 0 {
            sim.step();
        }
        if !shots.contains(&t) {
            continue;
        }
        let snap = sim.snapshot();
        for (name, mode) in [
            ("genetic", ColorMode::Genetic),
            ("strategy", ColorMode::Strategy),
            ("phase", ColorMode::Phase),
            ("energy", ColorMode::Energy),
        ] {
            renderer.set_mode(mode);
            let buf = renderer.render(&snap);
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for i in 0..(w * h) {
                out.extend_from_slice(&buf[i * 4..i * 4 + 3]);
            }
            std::fs::write(format!("{prefix}-t{t}-{name}.ppm"), out).expect("write frame");
        }
        println!(
            "{}",
            json!({ "tick": t, "pop": snap.stats.population, "wrote": format!("{prefix}-t{t}-*.ppm") })
        );
    }
}

/// Carrying capacity of the photosynthesis and vent niches, in cells, against
/// a lean specialist's upkeep, on an empty world (no shading yet).
pub fn niche(config: &WorldConfig) {
    let world = World::new(config);
    let (w, h) = (config.grid_width as usize, config.grid_height as usize);
    let mut lit_tiles = 0u32;
    let mut photo_total = 0f64;
    for tile in world.current_grid() {
        if tile.sunlight > 0 {
            lit_tiles += 1;
        }
        photo_total += config.photo_max_income as f64 * tile.sunlight as f64 / 255.0;
    }
    let vent_tiles = (0..h as u16)
        .flat_map(|y| (0..w as u16).map(move |x| (x, y)))
        .filter(|&(x, y)| world.is_in_vent_zone(x, y, config.vent_radius))
        .count();
    let vent_total = config.vent_count as f64 * config.vent_output as f64;
    let mut d = [8u8; genome::GENOME_LEN];
    d[genome::ENERGY_STORAGE_CAP] = 220;
    d[genome::TEMPERATURE_PREFERENCE] = 128;
    let upkeep = energy::metabolic_cost(&Genome::new(d).decode(config), 128, config) as f64;
    println!(
        "{}",
        json!({ "niche": {
            "upkeep_per_cell": r3(upkeep),
            "photo": { "lit_tiles": lit_tiles, "energy_per_tick": photo_total.round(), "cells": (photo_total / upkeep).round() },
            "thermo": { "vent_tiles": vent_tiles, "energy_per_tick": vent_total.round(), "cells": (vent_total / upkeep).round() },
            "grid_tiles": w * h,
        }})
    );
}
