//! Run mode: one JSON line per report interval describing the whole world.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use primordium::config::WorldConfig;
use primordium::sim::Simulation;
use primordium::sim::actions::{mapped_maturity_age, mapped_reproduction_threshold};
use primordium::sim::cell::Cell;
use primordium::sim::energy;
use primordium::sim::genome::{self, DecodedGenes};
use primordium::sim::phase::{TriggerCondition, apply_phase_modifiers, decode_threshold};
use primordium::sim::stats::{DeathCause, TickStats};

use crate::census::{self, CLASSES};
use crate::cli::{Args, r3};

/// Run one simulation, printing a report every `--every` ticks.
pub fn run(args: &Args, config: WorldConfig) {
    let ticks: u64 = args.parse("--ticks").unwrap_or(2000);
    let every = crate::cli::every(args, 100);
    let opts = Options {
        phase_detail: args.flag("--phase-detail"),
        traits: args.flag("--traits"),
    };

    let mut sim = Simulation::new(config);
    println!("{}", report(&sim, &TickStats::default(), 0, &opts));
    let mut window = TickStats::default();
    let mut since = 0;
    for t in 1..=ticks {
        sim.step();
        window.accumulate(&sim.world().stats);
        since += 1;
        let extinct = sim.world().population() == 0;
        // The last window can be shorter than --every (the run ends, or the
        // world dies): its rates are divided by the ticks it actually covers.
        if t % every == 0 || t == ticks || extinct {
            println!("{}", report(&sim, &window, since, &opts));
            window = TickStats::default();
            since = 0;
        }
        if extinct {
            println!("{}", json!({ "extinct_at": t }));
            break;
        }
    }
}

/// Decoded genes with the cell's active phase applied, which is what
/// `actions::decide` gates on.
fn effective(c: &Cell, d: &DecodedGenes) -> DecodedGenes {
    let mut e = d.clone();
    apply_phase_modifiers(&mut e, &c.genome, c.active_phase);
    e
}

pub struct Options {
    pub phase_detail: bool,
    pub traits: bool,
}

/// Name a phase trigger condition for the report.
pub fn cond_name(c: TriggerCondition) -> &'static str {
    match c {
        TriggerCondition::EnergyLow => "energy_low",
        TriggerCondition::EnergyHigh => "energy_high",
        TriggerCondition::ThreatNearby => "threat",
        TriggerCondition::KinNearby => "kin",
        TriggerCondition::AgeMature => "age",
        TriggerCondition::NoFood => "no_food",
        TriggerCondition::Crowded => "crowded",
        TriggerCondition::Wounded => "wounded",
    }
}

/// One report line. `window` is the number of ticks `stats` sums over (0 for
/// the tick-0 line, which has no flows yet).
pub fn report(sim: &Simulation, stats: &TickStats, window: u64, opts: &Options) -> Value {
    let world = sim.world();
    let config = sim.config();
    let cells = census::classified(sim);
    let census = census::census_of(world, config, &cells);
    let n = cells.len();
    let nf = n.max(1) as f64;
    let h = world.height as f64;

    let mut out = serde_json::Map::new();
    out.insert("tick".into(), json!(world.tick));
    out.insert("pop".into(), json!(n));

    // ── classes ──────────────────────────────────────────────────
    let mut class_y = [0f64; 5];
    let mut class_e = [0f64; 5];
    for (_, c, _, k) in &cells {
        class_y[*k] += c.position.1 as f64 / h;
        class_e[*k] += c.energy as f64;
    }
    let mut class = serde_json::Map::new();
    for k in 0..5 {
        let cn = census.class_n[k].max(1) as f64;
        class.insert(
            CLASSES[k].into(),
            json!([census.class_n[k], r3(class_y[k] / cn), r3(class_e[k] / cn)]),
        );
    }
    out.insert("class".into(), Value::Object(class));
    out.insert(
        "gene_class".into(),
        json!(
            CLASSES
                .iter()
                .zip(census.gene_class_n)
                .map(|(name, c)| (name.to_string(), json!(c)))
                .collect::<serde_json::Map<_, _>>()
        ),
    );
    out.insert("rows_90".into(), json!(census.rows_90));
    out.insert(
        "effective_groups".into(),
        json!(r3(census.effective_groups())),
    );

    out.insert("per_class".into(), per_class(sim, &cells));

    // ── lineages ─────────────────────────────────────────────────
    let mut lineages: BTreeMap<u32, u32> = BTreeMap::new();
    let mut xtab: BTreeMap<(u32, usize), u32> = BTreeMap::new();
    for (id, _, _, k) in &cells {
        let l = world.record(*id).lineage;
        *lineages.entry(l).or_default() += 1;
        *xtab.entry((l, *k)).or_default() += 1;
    }
    let mut lin: Vec<(u32, u32)> = lineages.into_iter().collect();
    lin.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: Vec<u32> = lin.iter().take(8).map(|&(l, _)| l).collect();
    out.insert("lineages".into(), json!(lin.len()));
    out.insert(
        "top_lin".into(),
        json!(lin.iter().take(6).map(|&(l, c)| [l, c]).collect::<Vec<_>>()),
    );
    // Lineage x class, for the eight largest lineages: a lineage's drift from
    // one way of living to another shows up here first.
    out.insert(
        "xtab".into(),
        json!(
            xtab.iter()
                .filter(|((l, _), _)| top.contains(l))
                .map(|(&(l, k), &c)| json!([l, CLASSES[k], c]))
                .collect::<Vec<_>>()
        ),
    );

    // ── energy, age, phase ───────────────────────────────────────
    let mut energy_sum = 0f64;
    let mut age_sum = 0f64;
    let mut cap_sum = 0f64;
    let mut phase_n = [0u32; 4];
    let mut phase_cond: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut mod_dev = 0f64;
    let mut active = 0u32;
    let mut slot0_always_on = 0u32;
    for (_, c, d, _) in &cells {
        energy_sum += c.energy as f64;
        age_sum += c.age as f64;
        cap_sum += energy::storage_cap(d, config) as f64;
        phase_n[c.active_phase.min(3) as usize] += 1;
        if c.active_phase > 0 {
            let slot = (c.active_phase - 1) as usize;
            let cond = TriggerCondition::from_byte(c.genome.phase_byte(slot, 0));
            *phase_cond.entry(cond_name(cond)).or_default() += 1;
            let dev: f64 = (2..6)
                .map(|off| (c.genome.phase_byte(slot, off) as f64 - 128.0).abs())
                .sum();
            mod_dev += dev / 4.0;
            active += 1;
        }
        if decode_threshold(c.genome.phase_byte(0, 1)).threshold == 0.0 {
            slot0_always_on += 1;
        }
    }
    out.insert("mean_energy".into(), json!(r3(energy_sum / nf)));
    out.insert("mean_cap".into(), json!(r3(cap_sum / nf)));
    out.insert("mean_age".into(), json!(r3(age_sum / nf)));
    out.insert("phase".into(), json!(phase_n));
    out.insert("phase_cond".into(), json!(phase_cond));
    out.insert(
        "non_default_phase".into(),
        json!(r3(census.non_default_phase)),
    );
    out.insert(
        "slot0_always_on".into(),
        json!(r3(slot0_always_on as f64 / nf)),
    );
    out.insert(
        "active_mod_dev".into(),
        json!(r3(mod_dev / active.max(1) as f64)),
    );

    // ── flows over the window ────────────────────────────────────
    if window > 0 {
        out.insert("per_tick".into(), per_tick(stats, window));
        let tot: u32 = stats.actions.iter().sum();
        let tf = tot.max(1) as f64;
        out.insert(
            "actions_pct".into(),
            json!(
                stats
                    .actions
                    .iter()
                    .map(|&a| r3(a as f64 * 100.0 / tf))
                    .collect::<Vec<_>>()
            ),
        );
        let es = stats.energy_samples.max(1) as f64;
        out.insert(
            "energy_per_cell_tick".into(),
            json!({
                "photo": r3(stats.income[0] / es),
                "thermo": r3(stats.income[1] / es),
                "scav": r3(stats.income[2] / es),
                "predation": r3(stats.income[3] / es),
                "metab": r3(stats.metabolism / es),
                "venom": r3(stats.venom / es),
                "toxin": r3(stats.toxin / es),
                "cap_waste": r3(stats.cap_waste / es),
                "dormant_frac": r3(stats.dormant as f64 / es),
                "combat_dmg": r3(stats.combat_damage / es),
                "shared": r3(stats.shared / es),
            }),
        );
        out.insert(
            "decay_dep_per_death".into(),
            json!(r3(
                stats.decay_deposited / stats.total_deaths().max(1) as f64
            )),
        );
        let total_ns: u64 = stats.phase_ns.iter().sum::<u64>().max(1);
        let names = [
            "diffusion",
            "sunlight",
            "prepare_next",
            "sense_decide",
            "resolve",
            "energy",
            "cleanup",
        ];
        out.insert(
            "phase_pct".into(),
            json!(
                names
                    .iter()
                    .zip(stats.phase_ns)
                    .map(|(name, ns)| (
                        name.to_string(),
                        json!(r3(ns as f64 * 100.0 / total_ns as f64))
                    ))
                    .collect::<serde_json::Map<_, _>>()
            ),
        );
        out.insert(
            "ms_per_tick".into(),
            json!(r3(total_ns as f64 / 1e6 / window as f64)),
        );
        let attacks = stats.attacks.max(1) as f64;
        let births = stats.births.max(1) as f64;
        out.insert(
            "genetics".into(),
            json!({
                "same_lineage_attack_frac": r3(stats.same_lineage_attacks as f64 / attacks),
                "mean_attack_gdist": r3(stats.attack_distance / attacks),
                "mean_child_gdist": (stats.child_distance / births * 1e4).round() / 1e4,
                "mean_child_mutated_bytes": r3(stats.child_mutated_bytes as f64 / births),
            }),
        );
    }

    out.insert("repro_gate".into(), repro_gate(config, &cells));
    out.insert("vents".into(), vents(sim, &cells));
    let (mismatch, ghosts) = world.integrity();
    out.insert(
        "integrity".into(),
        json!({ "pos_mismatch": mismatch, "ghosts": ghosts }),
    );

    // Mean raw byte and decoded value of the genes that decide strategy.
    let key = [0usize, 1, 3, 4, 5, 6, 12, 13, 17, 18, 21, 23, 32, 33];
    let mut raw = serde_json::Map::new();
    let mut dec = serde_json::Map::new();
    for &i in &key {
        let rsum: f64 = cells
            .iter()
            .map(|(_, c, _, _)| c.genome.data[i] as f64)
            .sum();
        let dsum: f64 = cells.iter().map(|(_, _, d, _)| d.get(i) as f64).sum();
        raw.insert(i.to_string(), json!((rsum / nf).round()));
        dec.insert(i.to_string(), json!(r3(dsum / nf)));
    }
    out.insert("raw".into(), Value::Object(raw));
    out.insert("dec".into(), Value::Object(dec));

    if opts.phase_detail {
        let mut hist = [0u32; 8];
        for (_, c, _, _) in &cells {
            for s in 0..genome::PHASE_SLOT_COUNT {
                hist[TriggerCondition::from_byte(c.genome.phase_byte(s, 0)) as usize] += 1;
            }
        }
        out.insert("slot_cond_hist".into(), json!(hist));
    }
    if opts.traits {
        out.insert("traits".into(), traits(sim, &cells, &top));
    }
    Value::Object(out)
}

fn per_tick(s: &TickStats, window: u64) -> Value {
    let w = window as f64;
    let mut deaths = serde_json::Map::new();
    for cause in DeathCause::ALL {
        deaths.insert(
            cause.name().into(),
            json!(r3(s.deaths[cause as usize] as f64 / w)),
        );
    }
    json!({
        "births": r3(s.births as f64 / w),
        "deaths": r3(s.total_deaths() as f64 / w),
        "death_causes": deaths,
        "kills": r3(s.kills() as f64 / w),
        "attacker_deaths": r3(s.attacker_deaths() as f64 / w),
        "attacks": r3(s.attacks as f64 / w),
        "attacks_missed": r3(s.attacks_missed as f64 / w),
        "blocked_repro": r3(s.repro_blocked as f64 / w),
        "phase_trans": r3(s.phase_transitions as f64 / w),
    })
}

/// Per class: what each strategy has earned and paid over its life (from the
/// sim's own records, per tick of age), what it stands on, and how close it
/// is to reproducing. Grid-wide means are dominated by the most numerous
/// class, so a class that is quietly starving is invisible in them.
fn per_class(sim: &Simulation, cells: &[(u32, &Cell, DecodedGenes, usize)]) -> Value {
    let world = sim.world();
    let config = sim.config();
    let mut n = [0u32; 5];
    let mut income = [0f64; 5];
    let mut upkeep = [0f64; 5];
    let mut tile_decay = [0f64; 5];
    let mut efrac = [0f64; 5];
    let mut speed = [0f64; 5];
    let mut ready = [0u32; 5];
    for (id, c, d, k) in cells {
        let r = world.record(*id);
        let age = c.age.max(1) as f64;
        n[*k] += 1;
        income[*k] += r.income.iter().map(|&v| v as f64).sum::<f64>() / age;
        upkeep[*k] += r.upkeep as f64 / age;
        tile_decay[*k] += world.current_tile(c.position.0, c.position.1).decay_energy as f64;
        let cap = energy::storage_cap(d, config);
        efrac[*k] += (c.energy / cap.max(1.0)) as f64;
        speed[*k] += d.get(genome::SPEED) as f64;
        let e = effective(c, d);
        if !is_dormant(c, &e, config) && c.energy >= mapped_reproduction_threshold(&e, config) {
            ready[*k] += 1;
        }
    }
    let mut out = serde_json::Map::new();
    for k in 0..5 {
        if n[k] == 0 {
            continue;
        }
        let f = n[k] as f64;
        out.insert(
            CLASSES[k].into(),
            json!({
                "n": n[k],
                "income": r3(income[k] / f),
                "upkeep": r3(upkeep[k] / f),
                "net": r3((income[k] - upkeep[k]) / f),
                "tile_decay": r3(tile_decay[k] / f),
                "energy_frac": r3(efrac[k] / f),
                "speed": r3(speed[k] / f),
                "repro_ready_pct": r3(ready[k] as f64 * 100.0 / f),
            }),
        );
    }
    Value::Object(out)
}

/// Whether `decide` would idle this cell as dormant (its gate 0).
fn is_dormant(c: &Cell, e: &DecodedGenes, config: &WorldConfig) -> bool {
    let cap = energy::storage_cap(e, config);
    let fraction = if cap > 0.0 { c.energy / cap } else { 0.0 };
    energy::dormancy_multiplier(e, fraction, config) < 1.0
}

/// Why cells are not reproducing, one gate at a time in `decide`'s order
/// (dormancy, then energy, cooldown, maturity), on the phase-modified genes
/// `decide` uses and the sim's own maturity and threshold rules (the old
/// harness hard-coded `gene * 1000` for maturity, which the sim had not used
/// since step 3). `eligible` cells may still lack an empty tile to breed into.
fn repro_gate(config: &WorldConfig, cells: &[(u32, &Cell, DecodedGenes, usize)]) -> Value {
    let (mut dormant, mut energy_b, mut cool, mut immature, mut ok, mut sterile) =
        (0, 0, 0, 0, 0, 0);
    let (mut mat_sum, mut life_sum, mut thr_sum) = (0f64, 0f64, 0f64);
    for (_, c, d, _) in cells {
        let e = effective(c, d);
        let thr = mapped_reproduction_threshold(&e, config);
        let mat = mapped_maturity_age(&e, config);
        let life = energy::lifespan_ticks(&e, config);
        mat_sum += mat as f64;
        life_sum += life as f64;
        thr_sum += thr as f64;
        if is_dormant(c, &e, config) {
            dormant += 1;
        } else if c.energy < thr {
            energy_b += 1;
        } else if c.cooldown_remaining > 0 {
            cool += 1;
        } else if c.age < mat {
            immature += 1;
        } else {
            ok += 1;
        }
        if mat >= life {
            sterile += 1;
        }
    }
    let nf = cells.len().max(1) as f64;
    json!({
        "blocked_dormant": dormant,
        "blocked_energy": energy_b,
        "blocked_cooldown": cool,
        "blocked_immature": immature,
        "eligible": ok,
        "sterile_by_design": sterile,
        "mean_maturity": (mat_sum / nf).round(),
        "mean_lifespan": (life_sum / nf).round(),
        "mean_threshold": r3(thr_sum / nf),
    })
}

/// How many cells stand in a vent zone (the sim's own definition), and how far
/// the nearest living cell is from a vent.
fn vents(sim: &Simulation, cells: &[(u32, &Cell, DecodedGenes, usize)]) -> Value {
    let world = sim.world();
    let config = sim.config();
    let bottom = world.height as i32 - 1;
    let mut in_zone = 0u32;
    let mut min_dist = i32::MAX;
    for (_, c, _, _) in cells {
        let (x, y) = c.position;
        if world.is_in_vent_zone(x, y, config.vent_radius) {
            in_zone += 1;
        }
        for &vx in &world.vent_positions {
            let dx = (x as i32 - vx as i32).abs();
            let dx = dx.min(world.width as i32 - dx);
            // Depth is measured from the bottom without wrapping (spec.md).
            let dy = bottom - y as i32;
            min_dist = min_dist.min(dx.max(dy));
        }
    }
    json!({
        "cells_in_zone": in_zone,
        "nearest_cell_distance": if min_dist == i32::MAX { -1 } else { min_dist },
    })
}

/// Per lineage, the traits a predator-prey brake depends on, so drift in any
/// of them is visible before the system crashes: attack and armour as the
/// sim applies them (decoded x255), the hunger slot (phase slot 0), maturity,
/// cooldown and predation efficiency. `bite_margin` is mean prey armour
/// (photo, thermo, scav) minus mean hunter attack, both sated: when it falls
/// to zero a sated predator starts killing again.
fn traits(sim: &Simulation, cells: &[(u32, &Cell, DecodedGenes, usize)], top: &[u32]) -> Value {
    let world = sim.world();
    let config = sim.config();
    let mut out = serde_json::Map::new();
    for &lin in top {
        let members: Vec<&(u32, &Cell, DecodedGenes, usize)> = cells
            .iter()
            .filter(|(id, _, _, _)| world.record(*id).lineage == lin)
            .collect();
        if members.len() < 5 {
            continue;
        }
        let m = members.len() as f64;
        let mean = |f: &dyn Fn(&Cell, &DecodedGenes) -> f64| -> f64 {
            r3(members.iter().map(|(_, c, d, _)| f(c, d)).sum::<f64>() / m)
        };
        let mut cond_votes = [0u32; 8];
        for (_, c, _, _) in &members {
            cond_votes[TriggerCondition::from_byte(c.genome.phase_byte(0, 0)) as usize] += 1;
        }
        let modal = (0..8).max_by_key(|&i| cond_votes[i]).unwrap_or(0);
        let mut class_votes = [0u32; 5];
        for (_, _, _, k) in &members {
            class_votes[*k] += 1;
        }
        let class = (0..5).max_by_key(|&i| class_votes[i]).unwrap_or(4);
        out.insert(
            lin.to_string(),
            json!({
                "n": members.len(),
                "class": CLASSES[class],
                "attack": mean(&|_, d| d.get(genome::ATTACK_POWER) as f64 * 255.0),
                "hungry_attack": mean(&|c, d| {
                    let mut h = d.clone();
                    apply_phase_modifiers(&mut h, &c.genome, 1);
                    h.get(genome::ATTACK_POWER) as f64 * 255.0
                }),
                "armor": mean(&|_, d| d.get(genome::ARMOR) as f64 * 255.0),
                "slot0_cond": cond_name(TriggerCondition::from_byte(modal as u8)),
                "slot0_cond_share": r3(cond_votes[modal] as f64 / m),
                "slot0_threshold": mean(&|c, _| c.genome.phase_byte(0, genome::PHASE_TRIGGER_THRESHOLD) as f64),
                "slot0_offense": mean(&|c, _| c.genome.phase_byte(0, genome::PHASE_OFFENSE_MOD) as f64),
                "maturity_ticks": mean(&|_, d| mapped_maturity_age(d, config) as f64),
                "cooldown_ticks": mean(&|_, d| primordium::sim::actions::mapped_reproduction_cooldown(d) as f64),
                "predation_eff": mean(&|_, d| d.get(genome::PREDATION_EFFICIENCY) as f64),
                "speed": mean(&|_, d| d.get(genome::SPEED) as f64),
                "raw_attack": mean(&|c, _| c.genome.data[genome::ATTACK_POWER] as f64),
                "raw_armor": mean(&|c, _| c.genome.data[genome::ARMOR] as f64),
            }),
        );
    }
    // Prey armour against the predators' sated bite, by class.
    let class_mean = |classes: &[usize], f: &dyn Fn(&DecodedGenes) -> f64| -> Option<f64> {
        let v: Vec<f64> = cells
            .iter()
            .filter(|(_, _, _, k)| classes.contains(k))
            .map(|(_, _, d, _)| f(d))
            .collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    };
    let prey_armor = class_mean(&[0, 1, 2], &|d| d.get(genome::ARMOR) as f64 * 255.0);
    let hunter_attack = class_mean(&[3], &|d| d.get(genome::ATTACK_POWER) as f64 * 255.0);
    if let (Some(a), Some(b)) = (prey_armor, hunter_attack) {
        out.insert("bite_margin".into(), json!(r3(a - b)));
    }
    Value::Object(out)
}
