// @veridikt
// kind: module
// name: Stats
// purpose: "Counts what happened during one tick, and keeps a per-cell record (founder lineage, income by channel, cause of death) that no simulation rule ever reads"
// owner: "primordium-maintainers"
// depends_on: World
// because: "A measurement taken by the code that moves the energy cannot drift from it. The lab harness used to recompute these numbers by hand from an out-of-tree patch, and four of its figures had quietly diverged from the sim (maturity, vent income, starvation, lineage)"

//! Instrumentation. Everything here is written by the simulation and read
//! only by observers (the lab harness, tests): no rule branches on it, so it
//! cannot change a run. `phase_ns` is wall-clock time and differs between
//! runs; every other field is deterministic.

use crate::sim::world::Strategy;

/// Why a cell died. Set by whatever killed it, read when the corpse is
/// cleaned up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathCause {
    /// Metabolism took energy to zero or below.
    Starvation,
    /// Venom ticking in its body finished it.
    Venom,
    /// Toxin on its tile finished it.
    Toxin,
    /// It outlived its genome's lifespan, however well fed.
    OldAge,
    /// Killed by a blow as the defender: a kill.
    Combat,
    /// Killed attacking, by the defender hitting back.
    Retaliation,
    /// Born with no energy: its parent's `offspring_energy_share` came to 0.
    /// The child still counts as a birth and still leaves a corpse. Only
    /// possible when `min_offspring_energy_share` is 0.
    Stillborn,
    /// Gave all its energy to its child (a share of 1). Only possible when
    /// `max_offspring_energy_share` is 1.
    Childbirth,
    /// Any other path to zero energy (none is known; counted so one would show).
    Other,
}

impl DeathCause {
    pub const ALL: [DeathCause; 9] = [
        DeathCause::Starvation,
        DeathCause::Venom,
        DeathCause::Toxin,
        DeathCause::OldAge,
        DeathCause::Combat,
        DeathCause::Retaliation,
        DeathCause::Stillborn,
        DeathCause::Childbirth,
        DeathCause::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DeathCause::Starvation => "starvation",
            DeathCause::Venom => "venom",
            DeathCause::Toxin => "toxin",
            DeathCause::OldAge => "old_age",
            DeathCause::Combat => "combat",
            DeathCause::Retaliation => "retaliation",
            DeathCause::Stillborn => "stillborn",
            DeathCause::Childbirth => "childbirth",
            DeathCause::Other => "other",
        }
    }
}

/// The channels a cell can earn energy through, in the order
/// `CellRecord::income` and `TickStats::income` use.
pub const INCOME_CHANNELS: [Strategy; 4] = [
    Strategy::Photosynthesis,
    Strategy::Thermosynthesis,
    Strategy::Scavenging,
    Strategy::Predation,
];

/// Index of each channel in `INCOME_CHANNELS`.
pub const PHOTO: usize = 0;
pub const THERMO: usize = 1;
pub const SCAVENGE: usize = 2;
pub const PREDATION: usize = 3;

/// A channel counts as a cell's diet only once it has paid at least this
/// share of the cell's lifetime upkeep. Below it the cell has not really
/// lived on anything yet — an unfed predator earns a trickle from a vestigial
/// photosynthesis gene, and would otherwise read as a photosynthesizer.
pub const DIET_MIN_UPKEEP_SHARE: f32 = 0.1;

/// Bookkeeping kept per cell, beside the cell pool rather than in `Cell`, so
/// the hot per-tick array stays small.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellRecord {
    /// Founder lineage, inherited unchanged by every descendant. Each founder
    /// (or cluster, or archetype band, or injected batch) gets its own id; 0
    /// means untagged.
    pub lineage: u32,
    /// Energy earned over the cell's life, per channel (`INCOME_CHANNELS`).
    pub income: [f32; 4],
    /// Metabolic cost paid over the cell's life.
    pub upkeep: f32,
    /// What killed it, once something has.
    pub death: Option<DeathCause>,
}

impl CellRecord {
    /// The channel that has paid this cell most over its life — what it
    /// actually lives on, as opposed to what its genes say it could do.
    /// `None` until one channel has paid `DIET_MIN_UPKEEP_SHARE` of its
    /// upkeep; callers fall back to the genome's strategy.
    pub fn diet(&self) -> Option<Strategy> {
        let (best, amount) =
            self.income.iter().enumerate().fold(
                (0, 0.0_f32),
                |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc },
            );
        if amount <= 0.0 || amount < DIET_MIN_UPKEEP_SHARE * self.upkeep {
            None
        } else {
            Some(INCOME_CHANNELS[best])
        }
    }
}

/// What happened during one tick, counted where it happened. Reset at the top
/// of every tick.
#[derive(Debug, Clone, Default)]
pub struct TickStats {
    /// Actions chosen, in priority order: Reproduce, Attack, Flee, Move,
    /// Share, Idle.
    pub actions: [u32; 6],
    pub births: u32,
    /// Reproductions refused because another birth already took the tile.
    pub repro_blocked: u32,
    pub attacks: u32,
    /// Attacks that missed because the target moved out of reach first
    /// (only with `flee_can_escape`). Not counted in `attacks`.
    pub attacks_missed: u32,
    /// Attacks whose attacker and defender share a founder lineage.
    pub same_lineage_attacks: u32,
    /// Sum over attacks of attacker-defender genetic distance.
    pub attack_distance: f64,
    /// Deaths by cause, indexed like `DeathCause::ALL`. Counted when the
    /// corpse is cleaned up, not when the blow lands: a cell struck to zero
    /// can still be revived in the same tick (by kin sharing energy with it,
    /// or by absorbing its own kill), and is then not dead. Kills are
    /// `deaths[Combat]`, attackers killed by retaliation `deaths[Retaliation]`.
    pub deaths: [u32; 9],
    /// Energy earned per channel (`INCOME_CHANNELS`), summed over cells.
    pub income: [f64; 4],
    pub metabolism: f64,
    pub venom: f64,
    pub toxin: f64,
    /// Energy above the storage cap, destroyed by the clamp.
    pub cap_waste: f64,
    /// Damage dealt in combat, both directions.
    pub combat_damage: f64,
    /// Energy handed from donors to kin.
    pub shared: f64,
    /// Cells whose metabolism ran at the dormant rate.
    pub dormant: u32,
    /// Cells settled in the energy phase (the denominator for the energy sums).
    pub energy_samples: u32,
    pub phase_transitions: u32,
    /// Decay matter deposited by corpses.
    pub decay_deposited: f64,
    /// Sum over births of parent-child genetic distance.
    pub child_distance: f64,
    /// Sum over births of bytes that differ between parent and child.
    pub child_mutated_bytes: u32,
    /// Wall time per tick phase, in nanoseconds: diffusion, sunlight,
    /// prepare_next, sense+decide, resolve, energy, cleanup. Not deterministic.
    pub phase_ns: [u64; 7],
}

impl TickStats {
    pub fn total_deaths(&self) -> u32 {
        self.deaths.iter().sum()
    }

    pub fn record_death(&mut self, cause: DeathCause) {
        self.deaths[cause as usize] += 1;
    }

    /// Defenders killed in combat.
    pub fn kills(&self) -> u32 {
        self.deaths[DeathCause::Combat as usize]
    }

    /// Attackers killed by the defender hitting back.
    pub fn attacker_deaths(&self) -> u32 {
        self.deaths[DeathCause::Retaliation as usize]
    }

    /// Add another tick's counts to these, for a report window.
    pub fn accumulate(&mut self, t: &TickStats) {
        for i in 0..6 {
            self.actions[i] += t.actions[i];
        }
        for i in 0..DeathCause::ALL.len() {
            self.deaths[i] += t.deaths[i];
        }
        for i in 0..4 {
            self.income[i] += t.income[i];
        }
        for i in 0..7 {
            self.phase_ns[i] += t.phase_ns[i];
        }
        self.births += t.births;
        self.repro_blocked += t.repro_blocked;
        self.attacks += t.attacks;
        self.attacks_missed += t.attacks_missed;
        self.same_lineage_attacks += t.same_lineage_attacks;
        self.attack_distance += t.attack_distance;
        self.metabolism += t.metabolism;
        self.venom += t.venom;
        self.toxin += t.toxin;
        self.cap_waste += t.cap_waste;
        self.combat_damage += t.combat_damage;
        self.shared += t.shared;
        self.dormant += t.dormant;
        self.energy_samples += t.energy_samples;
        self.phase_transitions += t.phase_transitions;
        self.decay_deposited += t.decay_deposited;
        self.child_distance += t.child_distance;
        self.child_mutated_bytes += t.child_mutated_bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diet_is_the_channel_that_paid_most_once_it_paid_enough() {
        let mut r = CellRecord {
            income: [2.0, 0.0, 30.0, 0.0],
            upkeep: 100.0,
            ..CellRecord::default()
        };
        assert_eq!(r.diet(), Some(Strategy::Scavenging));

        // A trickle that never covered a tenth of upkeep is not a diet.
        r.income = [2.0, 0.0, 0.0, 0.0];
        assert_eq!(r.diet(), None);

        // One kill outweighs a lifetime of trickle.
        r.income[PREDATION] = 110.0;
        assert_eq!(r.diet(), Some(Strategy::Predation));

        assert_eq!(CellRecord::default().diet(), None);
    }
}
