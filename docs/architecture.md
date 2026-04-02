# Primordium -- Architecture

## Data Layout

### Cell Storage

Cells live in a contiguous `Vec<Cell>` (the "cell pool"). The grid stores `u32` indices into this vec. Dead cells' indices go onto a free list. New cells take from the free list first, or push to the vec if empty.

Why not embed cells in tiles: most tiles are empty. A 1024x1024 grid has 1M tiles but typically 100-300K cells. Embedding wastes memory and trashes cache when iterating cells. A contiguous vec of living cells is cache-friendly for the per-tick loop.

#### Cell Struct

```
Cell {
    genome: [u8; 64]
    energy: f32
    age: u32
    position: (u16, u16)
    active_phase: u8            // 0 = default, 1-3 = phase slot index
    phase_ticks: u8             // ticks in current phase (for hysteresis)
    cooldown_remaining: u16     // reproduction cooldown counter
    venom_ticks: u8             // remaining poison damage ticks
    venom_damage: u8            // damage per poison tick
    last_damage_tick: u32       // for "wounded" phase trigger
    memory_dir: (i8, i8)        // remembered direction from sensing
}
```

~80 bytes per cell. 300K cells = ~24MB.

### Grid (Double-Buffered)

Two flat arrays of tiles. All cells read from "current" to make decisions. All writes go to "next." After the tick, swap pointers and clear "next."

#### Tile Struct

```
Tile {
    cell_id: u32          // index into cell pool, 0 = empty
    decay_energy: f32     // scavengeable remains
    pheromone: f32        // signal layer
    toxin: f32            // pollution layer
    temperature: u8       // static, set at init
    sunlight: u8          // static, derived from Y position
}
```

~18 bytes/tile. Two buffers at 1024x1024 = ~36MB.

### Diffusion Buffers

Two reusable `Vec<f32>` arrays (read/write). Shared across pheromone, toxin, and temperature diffusion passes run sequentially. 1M * 4 bytes * 2 = ~8MB.

### Total Memory Footprint (1024x1024, 300K cells)

| Component        | Size   |
|------------------|--------|
| Cell pool        | ~24MB  |
| Grid (2x)        | ~36MB  |
| Diffusion (2x)   | ~8MB   |
| **Total**        | **~68MB** |

---

## Module Structure

```
src/
  sim/
    mod.rs           -- Simulation struct, public API: new(), step(), snapshot()
    genome.rs        -- Genome struct, gene index constants, decode(),
                        mutate(), expression pipeline (antagonistic pairs,
                        top-N gating, phase modifiers -> effective stats)
    cell.rs          -- Cell struct, per-cell state
    world.rs         -- World struct: grid buffers, cell pool, free list,
                        environment layers, spatial queries
                        (neighbors_in_radius), tile access
    tick.rs          -- Tick orchestration: calls each phase in order,
                        manages buffer swaps
    actions.rs       -- Action enum (Move, Attack, Reproduce, Share, Idle),
                        decision logic, conflict resolution
    diffusion.rs     -- Generic field diffusion with per-layer config
    phase.rs         -- Phase evaluation, hysteresis tracking,
                        modifier computation
    energy.rs        -- Energy income (photo/thermo/scavenge),
                        metabolic drain, starvation
    spawner.rs       -- Initial seeding strategies, cell creation

  render/
    mod.rs           -- Renderer struct: takes WorldSnapshot,
                        produces pixel buffer
    color.rs         -- Genome-to-color mapping

  config.rs          -- WorldConfig struct (serde), loaded from JSON
  main.rs            -- Entry point: parse config, spawn threads,
                        window event loop
```

---

## Threading Model

```
  ┌─────────────────────────┐         ┌──────────────────────────┐
  │      Sim Thread          │         │     Main Thread          │
  │      (spawned)           │         │     (render + window)    │
  │                          │         │                          │
  │  loop {                  │         │  loop {                  │
  │    simulation.step()     │         │    read latest snapshot  │
  │    snapshot = world      │ ──────> │    render to pixel buf   │
  │      .snapshot()         │  swap   │    present frame         │
  │    publish(snapshot)     │         │    handle window events  │
  │  }                       │         │  }                       │
  └──────────────────────────┘         └──────────────────────────┘
```

The simulation runs on a spawned thread as fast as possible. The main thread owns the window event loop and renderer (required by macOS and some Linux windowing systems).

### Snapshot Transfer

The renderer only needs the latest completed frame. Old snapshots are discarded.

Strategy: triple-buffer or `arc-swap`. The sim writes to a back buffer, atomically swaps it into the "latest" slot. The renderer grabs the latest slot whenever it's ready to draw. Neither thread blocks.

#### WorldSnapshot

```
WorldSnapshot {
    tick: u64
    cells: Vec<(u16, u16, GenomeHash)>   // position + color data
    // optional overlay data:
    decay_map: Vec<f32>
    pheromone_map: Vec<f32>
    toxin_map: Vec<f32>
    stats: SimStats                       // population, avg energy, etc.
}
```

The snapshot is a lightweight projection, not a full world clone. Only data the renderer needs is copied.

---

## Tick Flow

Each call to `simulation.step()` executes these phases in order:

### Phase 1: Decay and Diffusion

- Diffuse pheromone field (fast decay, moderate spread, 8-neighbor)
- Diffuse toxin field (slow decay, slow spread, 4-neighbor)
- Diffuse temperature field (near-zero decay, very slow spread, 8-neighbor)
- Fade decay matter on all tiles

Each diffusion pass reads from one buffer, writes to the other, then swaps. The two diffusion buffers are reused across all three layers.

### Phase 2: Sensing

For each living cell:
- Read local tile state (sunlight, temperature, decay, toxin, pheromone)
- Scan neighbors within `sense_radius`
- Build a `SenseResult` (nearest food, nearest threat, kin count, pheromone gradient)
- Evaluate phase transition conditions against `SenseResult` and cell state
- Update `active_phase` with hysteresis checks

### Phase 3: Decision

For each living cell:
- Compute effective stats (raw genome -> antagonistic pairs -> top-N gating -> phase modifiers)
- Select action using hardcoded priority: **Reproduce > Attack > Flee > Move > Idle**
- Each action has a gate condition (e.g., reproduce only if energy > threshold and cooldown expired and target tile exists). First passing action wins.
- Record chosen action and target in an action buffer

### Phase 4: Action Resolution

Process all actions simultaneously against the current grid, writing results to the next grid:

- **Movement conflicts:** if two cells target the same tile, highest `rigidity` wins. Loser stays in place.
- **Attack resolution:** simultaneous damage exchange. Both attacker and defender take/deal damage in the same tick.
- **Reproduction:** child placed only if target tile is empty in the next grid. Parent and child energy split according to `offspring_energy_share`.
- **Resource sharing:** energy transferred to adjacent kin. Capped by donor's current energy.

### Phase 5: Energy Update

For each living cell in the next grid:
- Add photosynthesis income (based on local sunlight and effective `photosynthesis_rate`)
- Add thermosynthesis income (based on vent proximity and effective `thermosynthesis_rate`)
- Add scavenge income (based on tile decay matter and effective `scavenge_ability`)
- Subtract metabolic cost (base + expression cost from active genes)
- Apply venom tick damage if poisoned
- Apply toxin damage if on toxic tile (reduced by `toxin_resistance` and `membrane`)
- Cap energy at `energy_storage_cap`
- If energy <= 0: mark dead

### Phase 6: Cleanup

- Dead cells become decay matter on their tile (energy deposit = fraction of cell's energy at death)
- Generate toxin if deaths exceed `toxin_generation_threshold` in a local area
- Return dead cell indices to the free list
- Write pheromone contributions from living cells with `signal_emission`
- Swap current/next grid buffers
- Clear the new "next" buffer
- Increment tick counter

---

## Action Priority

Hardcoded priority order, evaluated top to bottom. First action whose gate condition passes is selected.

| Priority | Action    | Gate condition                                                                 |
|----------|-----------|--------------------------------------------------------------------------------|
| 1        | Reproduce | energy > reproduction_threshold AND cooldown expired AND empty adjacent tile exists AND age >= maturity_age |
| 2        | Attack    | hostile target within attack_range (genetic distance > aggression_trigger)     |
| 3        | Flee      | threat detected AND flee_response > 0 AND escape tile available               |
| 4        | Move      | speed check passes (random < speed/255) AND destination tile available         |
| 5        | Share     | kin adjacent AND resource_sharing check passes AND own energy above threshold  |
| 6        | Idle      | always passes (fallback)                                                       |

Note: Share is below Move intentionally. Sharing is altruistic and should not block self-preservation movement. Cells that evolve high `resource_sharing` will still share frequently because movement doesn't always trigger.

---

## Configuration

```
WorldConfig {
    // Grid
    grid_width: u32
    grid_height: u32

    // Energy sources
    sunlight_gradient_strength: f32
    vent_count: u32
    vent_output: f32
    vent_cycle: (u32, u32)          // (active_ticks, dormant_ticks)

    // Diffusion
    pheromone_decay: f32
    pheromone_diffusion: f32
    toxin_decay: f32
    toxin_diffusion: f32
    toxin_generation_threshold: u32

    // Decay
    decay_rate: f32

    // Temperature
    temperature_noise_scale: f32
    temperature_mismatch_cost: f32

    // Expression constraints
    top_n_gene_count: u32
    top_n_falloff: f32
    metabolic_cost_exponent: f32

    // Seeding
    initial_cell_count: u32
    initial_genome_strategy: SeedStrategy
    min_viable_acquisition: u8
    base_spawn_energy: f32
    bonus_spawn_energy: f32
    cluster_count: u32
    seed: u64

    // Simulation
    max_ticks: Option<u64>
}
```

Loaded from a JSON file at startup via `serde`. Immutable during a simulation run.

---

## Rendering Pipeline

1. Sim thread publishes a `WorldSnapshot` via atomic swap
2. Main thread grabs latest snapshot
3. Renderer iterates snapshot cells, writes RGBA pixels to a `Vec<u32>` framebuffer
4. Background tiles can optionally show environment overlays (sunlight gradient, temperature, pheromone heatmap)
5. Framebuffer is presented via the `pixels` crate to the window surface

Cell color is derived from a hash of the genome. The hash should be stable across mutations (small mutation = small color shift) so that lineages are visually trackable as gradual color drift.

---

## Determinism Guarantees

- All RNG uses a seeded `StdRng` (seed from config)
- Tick resolution is simultaneous (double-buffered), eliminating iteration order effects
- Floating point operations use consistent ordering (no parallel reductions with nondeterministic accumulation)
- Same config + same seed = identical simulation at any tick count

This enables:
- Exact replay of interesting runs
- A/B comparison: change one parameter, re-run from same seed, diff the outcomes
- Bug reproduction: save seed + config, share for debugging
