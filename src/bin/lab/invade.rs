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
use primordium::sim::actions::{mapped_maturity_age, mapped_reproduction_threshold};
use primordium::sim::cell::Cell;
use primordium::sim::energy;
use primordium::sim::genome::{GENOME_LEN, Genome};
use primordium::sim::spawner::{ARCHETYPES, Archetype, archetype_band_rows, archetype_genome};
use primordium::sim::stats::DeathCause;

use crate::census::{self, Census};
use crate::cli::{Args, die, mean_sd, parse_seeds, r3, run_parallel, slope};
use crate::report::{effective, is_dormant};

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

/// Where the invaders stand against `decide`'s reproduction gate, in
/// cell-ticks over the rare phase: which of maturity, cooldown and energy is
/// holding each one back. It tells "the life cycle brakes breeding" apart
/// from "it cannot afford to breed".
#[derive(Default, Clone, Copy)]
struct ReproStates {
    dormant: u64,
    immature: u64,
    /// Mature, over its threshold, waiting out its cooldown.
    cooldown_only: u64,
    cooldown_and_energy: u64,
    /// Mature, cooldown over, short of its threshold.
    energy_only: u64,
    /// Passes every gate; it breeds unless no tile in reach is empty.
    eligible: u64,
    /// Cells seen on the tick they reach their maturity age. Not a state:
    /// with the rare-phase juvenile deaths it gives survival to maturity.
    matured: u64,
    /// Cell-ticks at or past maturity, dormant or not.
    mature: u64,
}

impl ReproStates {
    const NAMES: [&'static str; 6] = [
        "dormant",
        "immature",
        "cooldown_only",
        "cooldown_and_energy",
        "energy_only",
        "eligible",
    ];

    fn counts(&self) -> [u64; 6] {
        [
            self.dormant,
            self.immature,
            self.cooldown_only,
            self.cooldown_and_energy,
            self.energy_only,
            self.eligible,
        ]
    }

    /// Classify one cell on the phase-modified genes `decide` gates on, in
    /// its order (dormancy first).
    fn add(&mut self, c: &Cell, config: &WorldConfig) {
        let e = effective(c, &c.genome.decode(config));
        let maturity = mapped_maturity_age(&e, config);
        if c.age == maturity {
            self.matured += 1;
        }
        if c.age >= maturity {
            self.mature += 1;
        }
        if is_dormant(c, &e, config) {
            self.dormant += 1;
            return;
        }
        if c.age < maturity {
            self.immature += 1;
            return;
        }
        let short = c.energy < mapped_reproduction_threshold(&e, config);
        match (c.cooldown_remaining > 0, short) {
            (true, false) => self.cooldown_only += 1,
            (true, true) => self.cooldown_and_energy += 1,
            (false, true) => self.energy_only += 1,
            (false, false) => self.eligible += 1,
        }
    }
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
    /// Invaders that died, over the whole run: their summed age at death, and
    /// how many died before their maturity age.
    dead_age_sum: u64,
    dead_immature: u32,
    repro_states: ReproStates,
    /// The rare phase as a life table, for `LifeTable::r0`.
    life: LifeTable,
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

    fn deaths_total(&self) -> u32 {
        self.death_causes.iter().sum()
    }
}

/// The invaders' rare phase as a life table. r while rare is a window
/// average: a lineage whose lifespan outlasts the window never pays its
/// old-age deaths inside it, so a long-lived invader reads as growing
/// whatever it does over a whole life. The expected births per lifetime do
/// not depend on the window.
#[derive(Default, Clone, Copy)]
struct LifeTable {
    births: u64,
    /// Invader-ticks spent at or past maturity.
    mature_ticks: u64,
    matured: u64,
    dead_immature: u64,
    /// Deaths at or past maturity from anything but old age.
    dead_adult: u64,
    /// Ticks from maturity to death by old age, for the introduced genome.
    adult_span: u32,
}

impl LifeTable {
    fn merge(&mut self, o: &LifeTable) {
        self.births += o.births;
        self.mature_ticks += o.mature_ticks;
        self.matured += o.matured;
        self.dead_immature += o.dead_immature;
        self.dead_adult += o.dead_adult;
        self.adult_span = o.adult_span;
    }

    /// Births per invader-tick past maturity.
    fn adult_birth_rate(&self) -> f64 {
        self.births as f64 / self.mature_ticks.max(1) as f64
    }

    /// Share of the cells seen either maturing or dying young that matured;
    /// 1 when none were seen (a lineage that is born mature has no juveniles).
    fn juvenile_survival(&self) -> f64 {
        match self.matured + self.dead_immature {
            0 => 1.0,
            seen => self.matured as f64 / seen as f64,
        }
    }

    /// Expected ticks an adult lives: `adult_span`, cut short at the adults'
    /// measured death rate from causes other than old age.
    fn adult_life(&self) -> f64 {
        let m = self.dead_adult as f64 / self.mature_ticks.max(1) as f64;
        let span = self.adult_span as f64;
        if m > 0.0 {
            (1.0 - (-m * span).exp()) / m
        } else {
            span
        }
    }

    /// Expected births per newborn over its whole life, taking the rare
    /// phase's adult birth rate, adult death rate and juvenile survival as
    /// constant. Above 1 the lineage replaces itself. Assumes the birth rate
    /// does not change with age, and means nothing for a boom: a lineage that
    /// passes the rare cap within a few ticks has no adult deaths measured.
    fn r0(&self) -> f64 {
        self.adult_birth_rate() * self.adult_life() * self.juvenile_survival()
    }

    fn json(&self) -> Value {
        json!({
            "adult_birth_rate": r5(self.adult_birth_rate()),
            "adult_life": r3(self.adult_life()),
            "adult_span": self.adult_span,
            "juvenile_survival": r3(self.juvenile_survival()),
            "r0_estimate": r3(self.r0()),
        })
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
    let (mut dead_age_sum, mut dead_immature) = (0u64, 0u32);
    let mut repro_states = ReproStates::default();
    let founder = plan.genome.decode(sim.config());
    let mut life = LifeTable {
        adult_span: energy::lifespan_ticks(&founder, sim.config())
            .saturating_sub(mapped_maturity_age(&founder, sim.config())),
        ..LifeTable::default()
    };
    let (mut xs, mut ys) = (
        vec![sim.world().tick as f64 / 100.0],
        vec![(placed.max(1) as f64).ln()],
    );
    // Keep running to --ticks even after the invader is gone, so the
    // residents' end state is compared at the same tick on every seed.
    while sim.world().tick < plan.ticks {
        if rare {
            for &id in &prev {
                repro_states.add(sim.world().get_cell(id), sim.config());
            }
        }
        sim.step();
        let t = sim.world().tick;
        let now = members(&sim);
        peak = peak.max(now.len());
        // A slot freed at this tick's cleanup is only reused by a birth next
        // tick, so the dead cell's record is still intact here.
        for &id in prev.difference(&now) {
            let r = sim.world().record(id);
            death_causes[r.death.unwrap_or(DeathCause::Other) as usize] += 1;
            // The cell's own fields survive `kill_cell` until the slot is
            // reused, so its age at death is still readable.
            let dead = sim.world().get_cell(id);
            dead_age_sum += dead.age as u64;
            let genes = dead.genome.decode(sim.config());
            let young = dead.age < mapped_maturity_age(&genes, sim.config());
            if young {
                dead_immature += 1;
            }
            if rare {
                if young {
                    life.dead_immature += 1;
                } else if r.death != Some(DeathCause::OldAge) {
                    life.dead_adult += 1;
                }
            }
            for (sum, &v) in dead_income.iter_mut().zip(&r.income) {
                *sum += v as f64;
            }
        }
        if rare && !prev.is_empty() {
            // A slot freed at one cleanup and reused by a birth in the same
            // lineage later is seen as absent for at least one sample, so
            // gross births and deaths are both counted.
            cell_ticks += prev.len() as u64;
            let born = now.difference(&prev).count() as u64;
            births += born;
            life.births += born;
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
    life.mature_ticks = repro_states.mature;
    life.matured = repro_states.matured;
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
        dead_age_sum,
        dead_immature,
        repro_states,
        life,
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
                "invader_mean_death_age": r3(r.dead_age_sum as f64 / (r.deaths_total().max(1)) as f64),
                "invader_dead_immature": r.dead_immature,
                "invader_repro_states": states_map(&r.repro_states.counts()),
                "invader_life_table": r.life.json(),
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
    // Pooled over seeds, so a seed with more invader cell-ticks weighs more.
    let mut pooled = [0u64; 6];
    let mut life = LifeTable::default();
    for r in &valid {
        life.merge(&r.life);
        for (sum, n) in pooled.iter_mut().zip(r.repro_states.counts()) {
            *sum += n;
        }
    }
    let dead: u32 = valid.iter().map(|r| r.deaths_total()).sum();
    let dead_immature: u32 = valid.iter().map(|r| r.dead_immature).sum();
    let dead_age: u64 = valid.iter().map(|r| r.dead_age_sum).sum();
    let extinct = valid.iter().filter(|r| r.extinct_at.is_some()).count();
    println!(
        "{}",
        json!({ "summary": {
            "invader": who, "rows": [rows.0, rows.1], "at": at, "n": n, "ticks": ticks,
            "rare_cap": plan.rare_cap, "seeds": valid.len(),
            "seeds_resident_extinct": runs.len() - valid.len(),
            "r_while_rare_mean": r5(m), "r_while_rare_se": r5(se),
            "seeds_grew": grew, "seeds_extinct": extinct,
            "invader_repro_states": states_map(&pooled),
            "invader_life_table": life.json(),
            "invader_mean_death_age": r3(dead_age as f64 / dead.max(1) as f64),
            "invader_dead_immature_share": r3(dead_immature as f64 / dead.max(1) as f64),
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

/// Each reproduction state's share of the cell-ticks counted.
fn states_map(counts: &[u64; 6]) -> Value {
    let total = counts.iter().sum::<u64>().max(1) as f64;
    json!(
        ReproStates::NAMES
            .iter()
            .zip(counts)
            .map(|(k, &n)| (k.to_string(), json!(r3(n as f64 / total))))
            .collect::<serde_json::Map<_, _>>()
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Each invader-tick lands under the gate that holds it back, in
    /// `decide`'s order, so the counts separate "the life cycle brakes
    /// breeding" from "it cannot afford to breed".
    #[test]
    fn an_invader_is_counted_under_the_gate_that_holds_it() {
        let config = WorldConfig {
            max_maturity_ticks: 300,
            ..WorldConfig::default()
        };
        let genome = archetype_genome(Archetype::Predator);
        let genes = genome.decode(&config);
        let maturity = mapped_maturity_age(&genes, &config);
        let threshold = mapped_reproduction_threshold(&genes, &config);
        let cell = |age: u32, energy: f32, cooldown: u16| {
            let mut c = Cell::new(genome.clone(), energy, (0, 0));
            c.age = age;
            c.cooldown_remaining = cooldown;
            c
        };

        let mut states = ReproStates::default();
        states.add(&cell(maturity - 1, threshold + 1.0, 0), &config);
        states.add(&cell(maturity, threshold + 1.0, 5), &config);
        states.add(&cell(maturity, threshold - 1.0, 5), &config);
        states.add(&cell(maturity, threshold - 1.0, 0), &config);
        // `decide` breeds at the threshold itself.
        states.add(&cell(maturity, threshold, 0), &config);
        assert_eq!(states.counts(), [0, 1, 1, 1, 1, 1]);
        assert_eq!(states.matured, 4, "four cells are on their maturity tick");
    }

    /// One birth per 100 adult ticks over a 200-tick adult life is two
    /// births per adult; three newborns in four reach maturity, so each
    /// newborn leaves 1.5. Adult deaths other than old age shorten the life.
    #[test]
    fn r0_is_births_per_adult_tick_times_adult_life_times_juvenile_survival() {
        let table = LifeTable {
            births: 10,
            mature_ticks: 1000,
            matured: 3,
            dead_immature: 1,
            dead_adult: 0,
            adult_span: 200,
        };
        assert!((table.r0() - 1.5).abs() < 1e-9, "r0 {}", table.r0());

        let dying = LifeTable {
            dead_adult: 5,
            ..table
        };
        assert!(dying.adult_life() < 200.0 && dying.r0() < table.r0());
    }
}
