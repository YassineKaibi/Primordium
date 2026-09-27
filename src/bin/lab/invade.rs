//! Invade mode: can a strategy grow from rare?
//!
//! Coexistence theory's operational test: let a resident community settle,
//! introduce a few cells of the focal strategy, and measure their per-capita
//! growth while they are still rare. If it is positive the strategy has a
//! niche under these mechanics, whatever the founders happened to be. That
//! separates "is this way of living viable" from "did the opening happen to
//! go its way", and it fits inside a short run.
//!
//! Usage: `lab --invade predator --config docs/handover/archetypes.json
//!         --set 'archetype_population_shares=[0.6,0.2,0.2,0]'
//!         --at 1000 --n 30 --ticks 1800 --seeds 1-5`
//! The invader is an archetype name or a 128-hex-digit genome.

use std::collections::HashSet;

use serde_json::{Value, json};

use primordium::config::WorldConfig;
use primordium::sim::Simulation;
use primordium::sim::genome::{GENOME_LEN, Genome};
use primordium::sim::spawner::{ARCHETYPES, Archetype, archetype_band_rows, archetype_genome};
use primordium::sim::stats::DeathCause;

use crate::census::{self, Census};
use crate::cli::{Args, die, mean_sd, parse_seeds, run_parallel, slope};

/// Resolve an archetype by name (or a short alias).
pub fn parse_archetype(name: &str) -> Option<Archetype> {
    let alias = match name {
        "photo" => "photosynthesizer",
        "vent" | "thermo" => "vent_feeder",
        "scav" => "scavenger",
        "pred" | "hunter" => "predator",
        other => other,
    };
    ARCHETYPES.iter().copied().find(|k| k.name() == alias)
}

fn parse_genome_hex(hex: &str) -> Option<Genome> {
    if hex.len() != GENOME_LEN * 2 {
        return None;
    }
    let mut d = [0u8; GENOME_LEN];
    for (i, byte) in d.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(Genome::new(d))
}

struct Invasion {
    seed: u64,
    /// The residents had died out before the introduction: nothing to invade.
    resident_extinct: bool,
    placed: usize,
    resident_pop: usize,
    n_end: usize,
    peak: usize,
    /// Growth while rare: from the introduction until the lineage first
    /// exceeds `rare_cap` cells, dies out, or the run ends. Births and deaths
    /// are gross flows per cell-tick; `r = births - deaths`, which over the
    /// rare phase equals the lineage's change divided by its cell-ticks, i.e.
    /// its mean per-capita growth rate while rare.
    rare_ticks: u64,
    births_pc: f64,
    deaths_pc: f64,
    /// It grew past `rare_cap`: the clearest possible positive result.
    reached_cap: bool,
    /// Slope of ln N per 100 ticks over the rare phase.
    log_slope_100: f64,
    extinct_at: Option<u64>,
    /// How the invaders died, over the whole run, indexed like
    /// `DeathCause::ALL`, and what the dead had lived on (lifetime income per
    /// channel, summed).
    death_causes: [u32; 9],
    dead_income: [f64; 4],
    /// The residents (every lineage but the invader's) just before the
    /// introduction and at the end of the run.
    resident_before: [u32; 5],
    resident_after: [u32; 5],
}

impl Invasion {
    fn r(&self) -> f64 {
        self.births_pc - self.deaths_pc
    }

    /// Grew from rare: reached the cap, or ended the rare phase with a
    /// positive per-capita rate.
    fn grew(&self) -> bool {
        self.reached_cap || self.r() > 0.0
    }
}

/// The census of everyone but `lineage`.
fn residents(sim: &Simulation, lineage: Option<u32>) -> Census {
    let cells: Vec<_> = census::classified(sim)
        .into_iter()
        .filter(|(id, _, _, _)| Some(sim.world().record(*id).lineage) != lineage)
        .collect();
    census::census_of(sim.world(), sim.config(), &cells)
}

struct Plan {
    genome: Genome,
    rows: (i32, usize),
    at: u64,
    n: usize,
    ticks: u64,
    rare_cap: usize,
}

fn run_one(config: &WorldConfig, seed: u64, plan: &Plan) -> Invasion {
    let mut config = config.clone();
    config.seed = seed;
    let mut sim = Simulation::new(config);
    while sim.world().tick < plan.at && sim.world().population() > 0 {
        sim.step();
    }
    let before = residents(&sim, None);
    let resident_extinct = before.pop == 0;
    let (lineage, placed) = sim.inject(&plan.genome, plan.n, plan.rows.0, plan.rows.1);

    let members = |sim: &Simulation| -> HashSet<u32> {
        let w = sim.world();
        w.live()
            .into_iter()
            .filter(|(id, _)| w.record(*id).lineage == lineage)
            .map(|(id, _)| id)
            .collect()
    };
    let mut prev = members(&sim);
    let (mut births, mut deaths, mut cell_ticks, mut rare_ticks) = (0u64, 0u64, 0u64, 0u64);
    let mut rare = true;
    let mut reached_cap = false;
    let mut peak = prev.len();
    let mut extinct_at = None;
    let mut death_causes = [0u32; 9];
    let mut dead_income = [0f64; 4];
    let (mut xs, mut ys) = (
        vec![sim.world().tick as f64 / 100.0],
        vec![(placed.max(1) as f64).ln()],
    );
    // Keep running to --ticks even after the invader is gone, so the
    // residents' end state is compared at the same tick on every seed.
    while sim.world().tick < plan.ticks {
        sim.step();
        let t = sim.world().tick;
        let now = members(&sim);
        peak = peak.max(now.len());
        // A slot freed at this tick's cleanup is only reused by a birth next
        // tick, so the dead cell's record is still intact here.
        for &id in prev.difference(&now) {
            let r = sim.world().record(id);
            death_causes[r.death.unwrap_or(DeathCause::Other) as usize] += 1;
            for (sum, &v) in dead_income.iter_mut().zip(&r.income) {
                *sum += v as f64;
            }
        }
        if rare && !prev.is_empty() {
            // A slot freed at one cleanup and reused by a birth in the same
            // lineage later is seen as absent for at least one sample, so
            // gross births and deaths are both counted.
            cell_ticks += prev.len() as u64;
            births += now.difference(&prev).count() as u64;
            deaths += prev.difference(&now).count() as u64;
            rare_ticks += 1;
            if !now.is_empty() {
                xs.push(t as f64 / 100.0);
                ys.push((now.len() as f64).ln());
            }
            if now.len() > plan.rare_cap {
                reached_cap = true;
                rare = false;
            }
        }
        if now.is_empty() && extinct_at.is_none() && !prev.is_empty() {
            extinct_at = Some(t);
            rare = false;
        }
        prev = now;
    }
    let after = residents(&sim, Some(lineage));
    let ct = cell_ticks.max(1) as f64;
    Invasion {
        seed,
        resident_extinct,
        placed,
        resident_pop: before.pop,
        n_end: prev.len(),
        peak,
        rare_ticks,
        births_pc: births as f64 / ct,
        deaths_pc: deaths as f64 / ct,
        reached_cap,
        log_slope_100: slope(&xs, &ys),
        extinct_at,
        death_causes,
        dead_income,
        resident_before: before.class_n,
        resident_after: after.class_n,
    }
}

pub fn run(args: &Args, config: WorldConfig) {
    let who = args
        .value("--invade")
        .unwrap_or_else(|| die("--invade needs a strategy"));
    let (genome, rows) = if let Some(kind) = parse_archetype(who) {
        (archetype_genome(kind), archetype_band_rows(&config, kind))
    } else if let Some(g) = parse_genome_hex(who) {
        (g, (0, config.grid_height as usize))
    } else {
        die(&format!(
            "--invade {who:?}: not an archetype (photo, vent, scav, pred) or a {}-digit hex genome",
            GENOME_LEN * 2
        ))
    };
    let rows = match args.value("--rows") {
        Some(spec) => {
            let (top, depth) = spec
                .split_once(',')
                .unwrap_or_else(|| die("--rows expects top,depth"));
            (
                top.parse().unwrap_or_else(|_| die("--rows: bad top")),
                depth.parse().unwrap_or_else(|_| die("--rows: bad depth")),
            )
        }
        None => rows,
    };
    let at: u64 = args.parse("--at").unwrap_or(1000);
    let n: usize = args.parse("--n").unwrap_or(30);
    let ticks: u64 = args.parse("--ticks").unwrap_or(at + 800);
    if ticks <= at {
        die("--ticks must be past --at");
    }
    let plan = Plan {
        genome,
        rows,
        at,
        n,
        ticks,
        // Still rare against a resident community of thousands.
        rare_cap: args.parse("--rare-cap").unwrap_or(5 * n),
    };
    let seeds = parse_seeds(args.value("--seeds").unwrap_or("1-5"));
    let jobs: usize = args.parse("--jobs").unwrap_or(3);

    let runs = run_parallel(seeds, jobs, |&seed| {
        let r = run_one(&config, seed, &plan);
        if r.resident_extinct {
            eprintln!("  seed {seed}: residents extinct before t={at}; left out of the summary");
        } else {
            eprintln!(
                "  seed {seed}: placed {} -> {} (peak {}), r while rare = {:+.5}/tick over {} ticks{}{}",
                r.placed,
                r.n_end,
                r.peak,
                r.r(),
                r.rare_ticks,
                if r.reached_cap {
                    ", passed the rare cap"
                } else {
                    ""
                },
                r.extinct_at
                    .map(|t| format!(", extinct at t={t}"))
                    .unwrap_or_default()
            );
        }
        r
    });

    for r in &runs {
        println!(
            "{}",
            json!({
                "seed": r.seed, "resident_extinct": r.resident_extinct, "placed": r.placed,
                "resident_pop": r.resident_pop, "n_end": r.n_end, "peak": r.peak,
                "endpoint_ratio": r5(r.n_end as f64 / r.placed.max(1) as f64),
                "rare_ticks": r.rare_ticks, "reached_rare_cap": r.reached_cap,
                "births_per_capita_tick": r5(r.births_pc),
                "deaths_per_capita_tick": r5(r.deaths_pc),
                "r_while_rare": r5(r.r()),
                "log_slope_per_100t": r5(r.log_slope_100),
                "extinct_at": r.extinct_at,
                "invader_death_causes": DeathCause::ALL
                    .iter()
                    .filter(|c| r.death_causes[**c as usize] > 0)
                    .map(|c| (c.name().to_string(), json!(r.death_causes[*c as usize])))
                    .collect::<serde_json::Map<_, _>>(),
                "dead_invader_income": {
                    "photo": r5(r.dead_income[0]), "thermo": r5(r.dead_income[1]),
                    "scav": r5(r.dead_income[2]), "predation": r5(r.dead_income[3]),
                },
                "resident_before": class_map(&r.resident_before),
                "resident_after": class_map(&r.resident_after),
            })
        );
    }
    let valid: Vec<&Invasion> = runs.iter().filter(|r| !r.resident_extinct).collect();
    let rs: Vec<f64> = valid.iter().map(|r| r.r()).collect();
    let (m, sd) = mean_sd(&rs);
    let se = if rs.is_empty() {
        0.0
    } else {
        sd / (rs.len() as f64).sqrt()
    };
    let grew = valid.iter().filter(|r| r.grew()).count();
    let extinct = valid.iter().filter(|r| r.extinct_at.is_some()).count();
    println!(
        "{}",
        json!({ "summary": {
            "invader": who, "rows": [rows.0, rows.1], "at": at, "n": n, "ticks": ticks,
            "rare_cap": plan.rare_cap, "seeds": valid.len(),
            "seeds_resident_extinct": runs.len() - valid.len(),
            "r_while_rare_mean": r5(m), "r_while_rare_se": r5(se),
            "seeds_grew": grew, "seeds_extinct": extinct,
        }})
    );
    eprintln!(
        "\ninvade {who}: r while rare = {:+.5} ± {:.5} per capita-tick over {} seeds; {} grew, {} went extinct",
        m,
        se,
        valid.len(),
        grew,
        extinct
    );
}

/// Per-capita rates are small; keep five decimals.
fn r5(x: f64) -> f64 {
    (x * 1e5).round() / 1e5
}

fn class_map(n: &[u32; 5]) -> Value {
    json!(
        census::CLASSES
            .iter()
            .zip(n)
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect::<serde_json::Map<_, _>>()
    )
}
