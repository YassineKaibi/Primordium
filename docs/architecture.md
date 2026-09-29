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

#### Cell Records (instrumentation)

Beside the pool, `World.records: Vec<CellRecord>` holds per-cell bookkeeping
that no simulation rule reads: the founder `lineage` (inherited unchanged by
every descendant), lifetime `income` per channel (photosynthesis, vent,
scavenging, predation), lifetime `upkeep`, and the `death` cause once
something has killed the cell. It is indexed like the pool and reset by
`spawn_cell`, so a recycled id never inherits a dead cell's record. It lives
outside `Cell` to keep the hot per-tick array small. `World.stats: TickStats`
counts what happened in the current tick (actions chosen, births, deaths by
cause, attacks and kills, energy flows, phase transitions, per-phase wall
time) and is reset at the top of every tick. See `sim/stats.rs`.

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
    sunlight: u8          // dynamic, Beer-Lambert column scan each tick
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
                        mutate(), expression pipeline (top-N gating, then
                        antagonistic pairs, then physical caps; phase
                        modifiers are applied after decode -> effective stats)
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
    spawner.rs       -- Initial seeding strategies, cell creation,
                        inject() for introducing cells mid-run (lab)
    stats.rs         -- Instrumentation: TickStats, CellRecord, DeathCause;
                        written by the sim, read only by observers

  render/
    mod.rs           -- Renderer struct: takes WorldSnapshot,
                        produces pixel buffer
    color.rs         -- Genome-to-color mapping

  config.rs          -- WorldConfig struct (serde), loaded from JSON;
                        fields a file leaves out take their defaults
  lib.rs             -- Library root: config, render, sim
  main.rs            -- Window binary: parse config, spawn threads,
                        window event loop
  bin/lab/           -- Headless measurement harness (`cargo run --release
                        --bin lab`): per-interval reports, multi-seed
                        scorecard with paired arms, invasion-from-rare
                        assay, archetype/niche/colour tools
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
    cells: Vec<CellView>                  // position, genome hash, strategy,
                                          // specialization, energy and age
                                          // fractions, active phase
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
- Recompute sunlight via Beer-Lambert column scan (per-tile α from cells, decay, toxin, pheromone)

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
- Compute effective stats (raw genome -> top-N gating -> antagonistic pairs -> physical caps -> phase modifiers). Action resolution and vent income recompute the same phase-modified values, so a phase applies to what a cell *does*, not only to what it decided.
- Select action using hardcoded priority: **Reproduce > Attack > Flee > Move > Idle**
- Each action has a gate condition (e.g., reproduce only if energy > threshold and cooldown expired and target tile exists). First passing action wins.
- Record chosen action and target in an action buffer

### Phase 4: Action Resolution

Process all actions simultaneously against the current grid, writing results to the next grid:

- **Movement conflicts:** if two cells target the same tile, highest `rigidity` wins. Loser stays in place.
- **Order:** Reproduce, then Attack, then Flee/Move, then Share, then Idle. With `flee_can_escape`, Flee/Move resolve before Attack, and a blow whose target has moved beyond the attacker's `attack_range` misses (`TickStats::attacks_missed`); otherwise the attack pass places the defender first and an attacked cell cannot move.
- **Attack resolution:** simultaneous damage exchange. Both attacker and defender take/deal damage in the same tick. If the defender dies, the attacker absorbs `predation_efficiency * max_predation_efficiency` of the victim's pre-blow energy; with `corpses_keep_energy` the rest is laid on the victim's tile as decay.
- **Reproduction:** child placed only if target tile is empty in the next grid. Parent and child energy split according to `offspring_energy_share`, bounded to `[min_offspring_energy_share, max_offspring_energy_share]` so neither leaves the split dead.
- **Resource sharing:** energy transferred to adjacent kin. Capped by donor's current energy.

### Phase 5: Energy Update

For each living cell in the next grid:
- Add photosynthesis income (based on local sunlight and effective `photosynthesis_rate`)
- Add thermosynthesis income (based on vent proximity and effective `thermosynthesis_rate`)
- Add scavenge income (based on tile decay matter and effective `scavenge_ability`)
- Subtract metabolic cost (summed per-gene expression cost x `metabolic_cost_scale`, plus the temperature-mismatch penalty; with `raw_genes_outside_expression` the raw-read genes are not summed)
- Apply venom tick damage if poisoned
- Apply toxin damage if on toxic tile (reduced by `toxin_resistance` and `membrane`)
- If age >= the cell's lifespan (from `max_age`): mark dead of old age; with `corpses_keep_energy`, lay `corpse_energy_fraction` of the energy it held on its tile
- Cap energy at the cell's storage capacity (`energy_cap_floor` + gene share of the range to `energy_cap_max`)
- If energy <= 0: mark dead

### Phase 6: Cleanup

- Dead cells become decay matter on their tile (`corpse_biomass`, grown with age when `corpse_growth_ticks` > 0, + `corpse_energy_fraction` of any energy left)
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
| 1        | Reproduce | energy > max(reproduction_threshold, `reproduction_energy_floor`) AND cooldown expired AND empty adjacent tile exists AND age >= maturity_age |
| 2        | Attack    | hostile target within attack_range (genetic distance > aggression_trigger). With `attack_only_when_harmful`: AND the blow beats the target's armour or the venom gets through its membrane. With `satiation_fraction` < 1: AND energy <= satiation_fraction x storage cap |
| 3        | Flee      | threat detected AND flee_response > 0 AND escape tile available. With `flee_can_escape`: a threat whose blow beats this cell's armour or whose venom gets through its membrane, AND random < speed * flee_response, AND escape tile available |
| 4        | Move      | speed check passes (random < speed/255, scaled by 1 - adhesion x kin share; with `foragers_stay_on_food` also by 1 - chemotaxis x own-tile food share) AND destination tile available |
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
    vent_radius: u32                // tiles a vent's output reaches
    vent_cycle: (u32, u32)          // (active_ticks, dormant_ticks)

    // Diffusion
    pheromone_decay: f32
    pheromone_diffusion: f32
    toxin_decay: f32
    toxin_diffusion: f32
    toxin_generation_threshold: u32

    // Decay
    decay_rate: f32
    decay_sink_rate: f32            // share of a tile's decay that sinks one row per tick
    initial_decay_matter: f32       // detritus on every tile at world creation
    corpse_decay_scale_min: f32
    corpse_decay_scale_max: f32

    // Temperature
    temperature_noise_scale: f32
    temperature_mismatch_cost: f32

    // Heredity
    max_mutation_rate: f32          // per-byte mutation chance at gene = 255
    max_mutation_magnitude: u8
    max_transposon_rate: f32
    max_adaptation_rate: f32        // thermal acclimation per tick at adaptation_rate 255
    max_move_distance: u32          // tiles per move; 1 = the spec's one tile per tick
    attack_only_when_harmful: bool  // attack only if the blow beats armour or the venom gets through the membrane
    satiation_fraction: f32         // above this share of its cap a cell starts no attack; 1.0 = off
    flee_can_escape: bool           // flee rolls speed, only from real threats; moves resolve before blows
    food_targets_richest: bool
    foragers_stay_on_food: bool     // a cell on food moves less, by chemotaxis x own-tile food share
    full_sense_range: bool          // sense_radius spans 1-4 tiles (ceil(gene*4)) instead of 1-3
    cell_light_absorption: f32      // Beer-Lambert alpha an occupied tile adds to its column (0.2)
    max_horizontal_transfer: f32      // largest byte shift at gene = 255

    // Lifecycle
    max_dormancy_trigger: f32
    min_dormancy_cost: f32
    min_lifespan_ticks: u32         // lifespan at max_age = 0
    max_lifespan_ticks: u32         // lifespan at max_age = 255
    max_maturity_ticks: u32         // maturity at maturity_age = 255
    maturity_lifespan_fraction: f32 // maturity is capped at this share of lifespan

    // Corpses
    corpse_biomass: f32             // structural matter every corpse leaves
    corpse_energy_fraction: f32     // share of remaining energy that becomes decay
    corpse_growth_ticks: u32        // ticks a body takes to reach full biomass; 0 = always full
    corpses_keep_energy: bool       // old corpses keep their energy; a kill's uneaten part becomes decay

    // Predation
    max_predation_efficiency: f32   // share of victim energy a kill pays at gene = 255

    // Expression constraints
    top_n_gene_count: u32
    top_n_falloff: f32
    metabolic_cost_exponent: f32
    metabolic_cost_scale: f32       // multiplier on summed expression cost
    raw_genes_outside_expression: bool // raw-read genes take no top-N slot, cost no upkeep

    // Energy economy
    photo_max_income: f32           // income of a perfect photosynthesizer in full sun
    scavenge_efficiency: f32        // fraction of consumed decay that becomes energy
    max_scavenge_per_tick: f32      // cap on decay stripped from one tile per tick
    energy_cap_floor: f32           // storage cap when energy_storage_cap gene = 0
    energy_cap_max: f32             // storage cap when energy_storage_cap gene = 1
    reproduction_energy_floor: f32  // absolute energy needed to split, whatever the cap
    min_offspring_energy_share: f32 // least share of parent energy a child gets
    max_offspring_energy_share: f32 // most share a parent can give away

    // Seeding
    initial_cell_count: u32
    initial_genome_strategy: SeedStrategy
    min_viable_acquisition: u8
    base_spawn_energy: f32
    bonus_spawn_energy: f32
    cluster_count: u32
    archetype_band_depth: u32       // rows per preset_archetypes band; 0 = derive from population
    archetype_population_shares: [f32; 4]  // photo / vent-feeder / scavenger / predator
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

Cell color comes from `render::color::cell_to_rgba(&CellView, ColorMode)`. A hash alone is not stable across mutations, so the mapping is strategy hue + bounded per-genome drift + energy brightness (see `docs/spec.md`, Visual Representation). `ColorMode` switches the view between `Genetic`, `Strategy`, `Phase` and `Energy`.

---

## Determinism Guarantees

- All RNG uses a seeded `ChaCha8Rng` (seed from config), including the lab's mid-run `inject`
- Instrumentation (`TickStats`, `CellRecord`) is write-only from the simulation's point of view: no rule branches on it. `TickStats::phase_ns` is wall-clock time and is the one non-deterministic field
- Tick resolution is simultaneous (double-buffered), eliminating iteration order effects
- Floating point operations use consistent ordering (no parallel reductions with nondeterministic accumulation)
- Same config + same seed = identical simulation at any tick count

This enables:
- Exact replay of interesting runs
- A/B comparison: change one parameter, re-run from same seed, diff the outcomes
- Bug reproduction: save seed + config, share for debugging
