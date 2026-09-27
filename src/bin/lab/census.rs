//! Who is alive, what they live on, and where: the per-sample measurement
//! every lab mode builds on.

use primordium::config::WorldConfig;
use primordium::sim::Simulation;
use primordium::sim::cell::Cell;
use primordium::sim::genome::DecodedGenes;
use primordium::sim::world::{Strategy, World, strategy_of};

/// Class names, indexed like `class_index`. The four functional groups come
/// first; `none` is a cell with no working acquisition channel.
pub const CLASSES: [&str; 5] = ["photo", "thermo", "scav", "hunter", "none"];
pub const GROUPS: usize = 4;

pub fn class_index(s: Strategy) -> usize {
    match s {
        Strategy::Photosynthesis => 0,
        Strategy::Thermosynthesis => 1,
        Strategy::Scavenging => 2,
        Strategy::Predation => 3,
        Strategy::None => 4,
    }
}

/// A cell's class by what it has actually lived on (`CellRecord::diet`),
/// falling back to what its genes are built for until one channel has paid
/// its way. The gene-cutoff classifier the old harness used filed the
/// hand-built predator under `none` and mixotrophs under whichever gene was
/// largest, however little it paid.
pub fn cell_class(world: &World, id: u32, decoded: &DecodedGenes) -> usize {
    let strategy = world
        .record(id)
        .diet()
        .unwrap_or_else(|| strategy_of(decoded).0);
    class_index(strategy)
}

/// One sample of the world.
#[derive(Debug, Clone, Default)]
pub struct Census {
    pub tick: u64,
    pub pop: usize,
    /// By diet (falling back to genes), indexed like `CLASSES`.
    pub class_n: [u32; 5],
    /// By genes alone, for comparison with earlier reports.
    pub gene_class_n: [u32; 5],
    /// Cells that have actually lived on each channel (a real diet, no gene
    /// fallback); index 4 counts cells that have no diet yet. A group is only
    /// *established* once enough cells live this way: at tick 0 nobody has
    /// earned anything, and a genome built for predation that lives on light
    /// is not a predator.
    pub diet_n: [u32; 5],
    /// The fewest rows that together hold 90% of the population: how thick
    /// the band of life is, whatever its shape or where it wraps.
    pub rows_90: u32,
    /// Share of cells in a non-default phase.
    pub non_default_phase: f64,
}

impl Census {
    /// Effective number of functional groups, `exp(Shannon entropy)` over the
    /// four groups: 1.0 is a monoculture, 4.0 four equal groups.
    pub fn effective_groups(&self) -> f64 {
        let total: f64 = self.class_n[..GROUPS].iter().map(|&n| n as f64).sum();
        if total == 0.0 {
            return 0.0;
        }
        let h: f64 = self.class_n[..GROUPS]
            .iter()
            .filter(|&&n| n > 0)
            .map(|&n| {
                let p = n as f64 / total;
                -p * p.ln()
            })
            .sum();
        h.exp()
    }
}

/// Every living cell with its decoded genes and class, decoded once.
pub fn classified(sim: &Simulation) -> Vec<(u32, &Cell, DecodedGenes, usize)> {
    let world = sim.world();
    let config = sim.config();
    world
        .live()
        .into_iter()
        .map(|(id, c)| {
            let d = c.genome.decode(config);
            let k = cell_class(world, id, &d);
            (id, c, d, k)
        })
        .collect()
}

pub fn take(sim: &Simulation) -> Census {
    census_of(sim.world(), sim.config(), &classified(sim))
}

pub fn census_of(
    world: &World,
    _config: &WorldConfig,
    cells: &[(u32, &Cell, DecodedGenes, usize)],
) -> Census {
    let mut c = Census {
        tick: world.tick,
        pop: cells.len(),
        ..Census::default()
    };
    let mut rows = vec![0u32; world.height as usize];
    let mut active = 0u32;
    for (id, cell, d, k) in cells {
        c.class_n[*k] += 1;
        c.diet_n[world.record(*id).diet().map(class_index).unwrap_or(4)] += 1;
        c.gene_class_n[class_index(strategy_of(d).0)] += 1;
        rows[cell.position.1 as usize] += 1;
        if cell.active_phase > 0 {
            active += 1;
        }
    }
    rows.sort_unstable_by(|a, b| b.cmp(a));
    let target = (c.pop as f64 * 0.9).ceil() as u32;
    let mut held = 0;
    for &n in &rows {
        if held >= target {
            break;
        }
        held += n;
        c.rows_90 += 1;
    }
    c.non_default_phase = active as f64 / c.pop.max(1) as f64;
    c
}
