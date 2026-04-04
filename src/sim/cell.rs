use crate::sim::genome::Genome;

/// A living cell in the simulation. Stored in the cell pool (Vec<Cell>).
/// Index 0 is reserved as "no cell" sentinel — real cells start at index 1.
#[derive(Debug, Clone)]
pub struct Cell {
    pub genome: Genome,
    pub energy: f32,
    pub age: u32,
    pub position: (u16, u16),
    /// 0 = default phase (no modifiers), 1-3 = active phase slot index
    pub active_phase: u8,
    /// Ticks spent in current phase (for hysteresis exit checks)
    pub phase_ticks: u8,
    /// Reproduction cooldown counter (decremented each tick, can reproduce at 0)
    pub cooldown_remaining: u16,
    /// Remaining poison damage ticks from venom
    pub venom_ticks: u8,
    /// Damage dealt per venom tick
    pub venom_damage: u8,
    /// Tick number when this cell last took damage (for "wounded" trigger)
    pub last_damage_tick: u32,
    /// Remembered direction from sensing (persists for `memory_length` ticks)
    pub memory_dir: (i8, i8),
}

impl Cell {
    /// Create a new cell with the given genome, energy, and position.
    /// All combat/phase state starts zeroed out.
    pub fn new(genome: Genome, energy: f32, position: (u16, u16)) -> Self {
        Self {
            genome,
            energy,
            age: 0,
            position,
            active_phase: 0,
            phase_ticks: 0,
            cooldown_remaining: 0,
            venom_ticks: 0,
            venom_damage: 0,
            last_damage_tick: 0,
            memory_dir: (0, 0),
        }
    }

    /// A cell is alive when its energy is positive.
    #[inline]
    pub fn is_alive(&self) -> bool {
        self.energy > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::genome::GENOME_LEN;

    fn make_cell() -> Cell {
        let genome = Genome::new([0u8; GENOME_LEN]);
        Cell::new(genome, 100.0, (5, 10))
    }

    #[test]
    fn cell_struct_size_under_96_bytes() {
        // Architecture doc targets ~80 bytes. We allow some padding slack
        // but flag if it balloons beyond 96.
        let size = std::mem::size_of::<Cell>();
        assert!(
            size <= 96,
            "Cell struct is {size} bytes — expected ≤96 (target ~80)"
        );
    }

    #[test]
    fn new_cell_defaults() {
        let c = make_cell();
        assert_eq!(c.age, 0);
        assert_eq!(c.active_phase, 0);
        assert_eq!(c.phase_ticks, 0);
        assert_eq!(c.cooldown_remaining, 0);
        assert_eq!(c.venom_ticks, 0);
        assert_eq!(c.venom_damage, 0);
        assert_eq!(c.last_damage_tick, 0);
        assert_eq!(c.memory_dir, (0, 0));
        assert_eq!(c.position, (5, 10));
        assert!((c.energy - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn is_alive_positive_energy() {
        let c = make_cell();
        assert!(c.is_alive());
    }

    #[test]
    fn is_alive_zero_energy() {
        let mut c = make_cell();
        c.energy = 0.0;
        assert!(!c.is_alive());
    }

    #[test]
    fn is_alive_negative_energy() {
        let mut c = make_cell();
        c.energy = -1.0;
        assert!(!c.is_alive());
    }
}
