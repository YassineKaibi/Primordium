# Primordium

An evolutionary cell simulator where life emerges from scratch. Each cell is a single pixel on a 2D grid, carrying a 64-byte genome that encodes metabolism, movement, combat, reproduction, sensing, and social behavior. There are no predefined species -- all behavioral diversity arises from natural selection acting on random mutations under environmental pressure.

Built in Rust for deterministic, reproducible simulations.

## What Happens

Cells compete for energy on a toroidal grid with two resource axes: sunlight from above and thermal vents from below. Over thousands of ticks, populations diverge into distinct ecological niches.

**Emergent behaviors observed:**
- Photosynthesizer colonies clustering near the surface with adhesion and resource sharing
- Fast predators with chemotaxis hunting in the middle zone
- Armored venomous cells holding territory
- Swarm hunters coordinating via pheromone signaling
- Dormant cells hibernating through resource scarcity
- Thermosynthetic specialists colonizing the deep vent zone

No behavior is hardcoded. Everything emerges from genome expression, environmental constraints, and selection pressure.

## Core Mechanics

**Genome** -- 64 bytes encoding 46 base genes across 10 functional groups (metabolism, movement, combat, reproduction, sensing, structural, lifecycle, environmental, social, meta-evolution) plus 3 conditional phase slots that modify gene expression based on environmental triggers.

**Expression constraints** prevent convergence toward homogeneous supercells:
- Superlinear metabolic cost curve (maxing genes drains energy faster than it can be acquired)
- Top-N gating (only ~10-12 of 46 genes express at full strength, forcing specialization)
- Antagonistic gene pairs (photosynthesis vs speed, armor vs speed, attack power vs range)

**Environment** -- sunlight gradient, thermal vents, temperature biomes (Perlin noise), dynamic toxin/pheromone/decay fields with diffusion. Three energy acquisition channels: photosynthesis, thermosynthesis, and scavenging.

**Determinism** -- simultaneous tick resolution via double-buffered grids, seeded RNG. Same seed and config always produce the same simulation.

## Building

### Prerequisites

- Rust (latest stable)

### Run

```bash
cargo run --release
```

### Test

```bash
cargo test
```

### Lint

```bash
cargo clippy -- -D warnings
cargo fmt -- --check
```

## Configuration

World parameters are defined in a JSON config file loaded at startup. Key parameters include grid dimensions, sunlight gradient strength, vent count and output, diffusion rates, expression constraint tuning (top-N count, metabolic cost exponent), initial population size, seeding strategy, and RNG seed.

See `docs/spec.md` for the full parameter table.

## Documentation

| Document | Contents |
|----------|----------|
| `docs/spec.md` | Complete genome specification (all 46 genes, phase table, expression constraints) and environment design (energy sources, diffusion, tick order, world parameters) |
| `docs/architecture.md` | Data layout, module structure, threading model, tick flow, action priority, memory footprint, determinism guarantees |
| `CLAUDE.md` | Development conventions (branching, commits, code style, testing, key invariants) |

## Architecture

The simulation runs on a dedicated thread, decoupled from rendering. The renderer grabs the latest world snapshot via atomic swap and draws to a pixel buffer on the main thread.

```
Sim Thread                            Main Thread
  loop {                                loop {
    step()            ── snapshot ──>     render latest snapshot
    publish snapshot                      present frame
  }                                       handle window events
                                        }
```

Cell data lives in a contiguous pool (~80 bytes/cell) indexed by the grid. The grid is double-buffered (~18 bytes/tile x2). Total memory footprint at 1024x1024 with 300K cells is ~68MB.

## License

MIT
