# Primordium -- Evolutionary Cell Simulator Specification

## Overview

Primordium is a pixel-based evolutionary simulation written in Rust. Each cell occupies a single pixel on a 2D toroidal grid and carries a 64-byte genome that encodes its traits, behaviors, and phase-transition rules. There are no hardcoded species -- all behavioral diversity emerges from natural selection acting on random mutations under environmental pressure.

The simulation is deterministic: identical seeds produce identical outcomes. This is achieved through simultaneous tick resolution with double-buffered grids.

---

## Genome Specification

**Total size: 64 bytes per cell**
- 46 base genes (1 byte each, `u8`, range 0-255)
- 3 phase slots (6 bytes each = 18 bytes)

### Base Genes (46 bytes)

#### Metabolism (6 genes)

| Index | Gene                   | Description                                                                 |
|-------|------------------------|-----------------------------------------------------------------------------|
| 0     | `photosynthesis_rate`  | Energy gained from sunlight per tick. Effective only in lit zones.           |
| 1     | `thermosynthesis_rate` | Energy gained from thermal vents. Effective only near vents.                |
| 2     | `predation_efficiency` | Percentage of victim's energy absorbed on kill.                             |
| 3     | `scavenge_ability`     | Energy extracted from decay matter on a tile.                               |
| 4     | `energy_storage_cap`   | Maximum energy the cell can hold. Excess is wasted.                         |
| 5     | `base_metabolism`      | Passive energy drain per tick. Lower is more efficient, but antagonistic pairs and active gene expression push effective cost higher. |

#### Movement (6 genes)

| Index | Gene                  | Description                                                             |
|-------|-----------------------|-------------------------------------------------------------------------|
| 6     | `speed`               | Probability of moving each tick. 0 = sessile, 255 = moves every tick.   |
| 7     | `direction_bias`      | Preferred heading. 0-255 maps linearly to 0-360 degrees.               |
| 8     | `direction_noise`     | Randomness added to movement direction. Low = straight lines, high = Brownian. |
| 9     | `chemotaxis_strength` | Tendency to move toward nearby energy sources.                          |
| 10    | `flee_response`       | Tendency to move away from larger or aggressive neighbors.              |
| 11    | `pack_affinity`       | Tendency to move toward genetically similar cells.                      |

#### Combat (5 genes)

| Index | Gene                 | Description                                                               |
|-------|----------------------|---------------------------------------------------------------------------|
| 12    | `attack_power`       | Damage dealt on collision or attack action.                               |
| 13    | `armor`              | Flat damage reduction when attacked.                                      |
| 14    | `venom`              | Delayed poison: deals damage to target over N ticks after contact.        |
| 15    | `attack_range`       | Attack reach in pixels (1-3). Capped by `sense_radius`. Higher costs more energy per attack. |
| 16    | `aggression_trigger` | Genetic distance threshold for attacking. Low = attacks everything, high = attacks only very different cells. |

#### Reproduction (6 genes)

| Index | Gene                     | Description                                                         |
|-------|--------------------------|---------------------------------------------------------------------|
| 17    | `reproduction_threshold` | Energy level at which the cell splits.                              |
| 18    | `offspring_energy_share` | Percentage of parent energy transferred to offspring.               |
| 19    | `mutation_rate`          | Per-gene probability of mutation during reproduction.               |
| 20    | `mutation_magnitude`     | Maximum shift applied to a mutated gene value.                      |
| 21    | `reproduction_cooldown`  | Minimum ticks between successive reproductions.                     |
| 22    | `offspring_scatter`      | Distance from parent at which offspring spawns. Capped by `sense_radius`. |

#### Sensing (5 genes)

| Index | Gene                 | Description                                                              |
|-------|----------------------|--------------------------------------------------------------------------|
| 23    | `sense_radius`       | Detection range in pixels (1-4). Higher values add metabolic cost.       |
| 24    | `sense_priority`     | What the cell prioritizes detecting. 0 = food, 255 = threats. Gradient.  |
| 25    | `memory_length`      | Ticks of directional memory. 0 = purely reactive.                        |
| 26    | `signal_emission`    | Pheromone output strength per tick. Adds to local pheromone layer.        |
| 27    | `signal_sensitivity` | Ability to detect pheromone concentrations on nearby tiles.              |

#### Structural (4 genes)

| Index | Gene        | Description                                                                    |
|-------|-------------|--------------------------------------------------------------------------------|
| 28    | `adhesion`  | Tendency to stick to adjacent genetically similar cells. Enables cluster formation. |
| 29    | `rigidity`  | Resistance to being displaced or pushed by other cells.                        |
| 30    | `decay_rate`| How long the cell's corpse persists as scavengeable decay matter.              |
| 31    | `membrane`  | Resistance to venom and environmental damage (toxin).                          |

#### Lifecycle (4 genes)

| Index | Gene               | Description                                                              |
|-------|--------------------|--------------------------------------------------------------------------|
| 32    | `max_age`          | Tick count before natural death.                                         |
| 33    | `maturity_age`     | Ticks before reproduction is unlocked.                                   |
| 34    | `dormancy_trigger` | Energy threshold below which the cell enters dormancy phase.             |
| 35    | `dormancy_cost`    | Energy drain rate while in dormancy (lower = better hibernation).        |

#### Environmental (3 genes)

| Index | Gene                   | Description                                                          |
|-------|------------------------|----------------------------------------------------------------------|
| 36    | `temperature_preference`| Optimal temperature zone. Mismatch with local temp = extra metabolism cost. |
| 37    | `toxin_resistance`     | Damage reduction from toxin exposure on polluted tiles.               |
| 38    | `adaptation_rate`      | Speed of within-lifetime epigenetic-like modifier shifts. Not inherited. |

#### Social (4 genes)

| Index | Gene                        | Description                                                       |
|-------|-----------------------------|-------------------------------------------------------------------|
| 39    | `kin_recognition_precision` | Accuracy of genetic similarity detection.                         |
| 40    | `resource_sharing`          | Probability of transferring energy to adjacent kin.               |
| 41    | `territorial_radius`       | Radius of area the cell defends. Attacks non-kin who enter. Capped by `speed`. |
| 42    | `swarm_signal`              | Emits a rally pheromone when food is found.                       |

#### Meta (3 genes)

| Index | Gene                  | Description                                                             |
|-------|-----------------------|-------------------------------------------------------------------------|
| 43    | `gene_linkage`        | Controls which gene clusters tend to mutate together (simulates chromosomes). |
| 44    | `horizontal_transfer` | Probability of absorbing genes from consumed cells into own genome.     |
| 45    | `transposon_rate`     | Rate of internal gene duplication and shuffling within the genome.      |

---

### Phase Table (18 bytes)

3 phase slots, 6 bytes each. Phases modify gene expression based on environmental conditions without changing the genome itself.

**Phase resolution order:** slots are evaluated 0, 1, 2. First match wins. No match = default active state (no modifiers applied).

Evolution can disable a slot by setting `trigger_threshold` to 0 or 255 depending on condition polarity.

#### Phase Slot Layout (6 bytes)

| Byte | Field               | Description                                                                |
|------|---------------------|----------------------------------------------------------------------------|
| 0    | `trigger_condition` | Enum selecting what triggers this phase. See condition table below.        |
| 1    | `trigger_threshold` | Upper 6 bits (0-63, mapped to full range): activation value. Lower 2 bits: hysteresis band preset. |
| 2    | `offense_mod`       | Combat gene modifier. 128 = neutral, <128 = suppress, >128 = boost.       |
| 3    | `defense_mod`       | Armor/membrane/rigidity modifier. Same scale.                              |
| 4    | `mobility_mod`      | Speed/chemotaxis/flee modifier. Same scale.                                |
| 5    | `efficiency_mod`    | Metabolism/sensing cost modifier. Same scale.                              |

#### Trigger Conditions

| Value | Condition       | Fires when                                    |
|-------|-----------------|-----------------------------------------------|
| 0     | `energy_low`    | Cell energy below threshold                   |
| 1     | `energy_high`   | Cell energy above threshold                   |
| 2     | `threat_nearby` | Aggressive non-kin within sense radius         |
| 3     | `kin_nearby`    | Genetic kin count within sense radius > threshold |
| 4     | `age_mature`    | Cell age exceeds threshold                     |
| 5     | `no_food`       | No energy source detected within sense radius  |
| 6     | `crowded`       | Neighbor count exceeds threshold               |
| 7     | `wounded`       | Cell has taken damage recently (within N ticks) |

#### Hysteresis Presets (lower 2 bits of `trigger_threshold`)

| Value | Band size            | Intended use                        |
|-------|----------------------|-------------------------------------|
| 0     | 0 (none)             | Immediate reactions: flee, attack   |
| 1     | ~10% of threshold    | Light smoothing                     |
| 2     | ~25% of threshold    | Moderate commitment: foraging shift |
| 3     | ~40% of threshold    | Strong commitment: dormancy         |

Entry happens when the condition crosses the threshold. Exit requires crossing threshold + band. This prevents rapid phase flickering.

---

### Expression Constraints

Three mechanisms prevent convergence toward homogeneous "supercells."

#### 1. Metabolic Budget

Every gene has an expression cost. Total expression cost = effective per-tick energy drain. Costs scale **superlinearly** (exponent 1.5-2.0): pushing a gene from 50% to 100% effectiveness costs disproportionately more than 0% to 50%.

A cell with all genes maxed drains energy faster than any acquisition method can replenish.

#### 2. Top-N Gating (Specialization Pressure)

The genome has a finite expression capacity. The top N highest genes (N ~ 10-12 out of 46) express at full value. Genes ranked below the Nth are steeply attenuated.

This forces evolutionary specialization: a cell can invest in ~10-12 strong traits, but cannot have 30+ high traits simultaneously. The choice of which genes to invest in defines the cell's ecological niche.

Note: entropy-based penalty (continuous alternative) is planned for a later iteration.

#### 3. Antagonistic Gene Pairs

Hardcoded relationships applied at decode time. Both genes can be encoded high, but the effective value of each is reduced by the other.

| Gene A                  | Gene B              | Relationship                                    |
|-------------------------|---------------------|-------------------------------------------------|
| `photosynthesis_rate`   | `speed`             | Plants don't run. effective_photo = photo * (1 - speed * 0.7) |
| `photosynthesis_rate`   | `thermosynthesis_rate` | Distinct energy strategies. Investing in both penalizes each. |
| `armor`                 | `speed`             | Heavy defense slows movement.                   |
| `attack_power`          | `energy_storage_cap`| Weapons reduce storage capacity.                |
| `sense_radius`          | `base_metabolism`   | Awareness increases metabolic drain.            |
| `adhesion`              | `speed`             | Stuck cells can't chase prey.                   |
| `signal_emission`       | `base_metabolism`   | Broadcasting is energetically expensive.        |
| `territorial_radius`    | `pack_affinity`     | Loners vs swarmers.                             |
| `attack_range`          | `attack_power`      | Ranged attacks are weaker.                      |

#### Physical Caps

Some genes are structurally capped by others:
- `attack_range` <= `sense_radius` (can't hit what you can't see)
- `territorial_radius` capped by `speed` (can't patrol unreachable area)
- `offspring_scatter` capped by `sense_radius` (can't place offspring beyond perception)

---

### Visual Representation

Cell color is derived from genome hash. Genetically similar cells appear visually similar. Speciation is visible as color clustering on the grid.

---

## Environment Specification

### Grid

- **Topology:** 2D toroidal (wraps on both axes, no edges)
- **Tile contents:** at most one living cell, plus environmental state
- **Resolution target:** 1024x1024 (1M tiles), configurable

### Tile Data (per tile)

| Field          | Type | Description                                   |
|----------------|------|-----------------------------------------------|
| `cell_id`      | u32  | Index into cell array. 0 = empty.             |
| `decay_energy` | f32  | Scavengeable remains from dead cells.         |
| `pheromone`    | f32  | Signal layer written by cells, decays per tick.|
| `temperature`  | u8   | Static (Perlin noise at init), rarely changes. |
| `toxin`        | f32  | Dynamic. Generated by death clusters, decays slowly. |
| `sunlight`     | u8   | Static. Derived from Y position (gradient).   |

Approximate memory: ~18 bytes/tile. At 1024x1024 = ~18MB per buffer, ~36MB with double buffering.

### Energy Sources

Three channels for energy entering the system:

#### Sunlight

Vertical gradient across the Y axis. Top rows receive maximum energy, bottom rows receive minimal or zero. Photosynthesizers absorb energy proportional to `photosynthesis_rate` and local sunlight intensity.

Creates a "surface" zone where photosynthetic cells thrive.

#### Thermal Vents

Localized high-energy sources positioned along the **bottom edge** of the grid. Fixed positions, finite output per tick shared among adjacent cells.

Can be configured as:
- **Permanent:** constant output
- **Periodic:** erupt for N ticks, dormant for M ticks

Cells absorb vent energy proportional to `thermosynthesis_rate`. This creates a distinct bottom-dwelling ecological niche, separate from photosynthesizers.

The vertical resource axis:
- **Top zone:** high sunlight, no vents. Photosynthesizers dominate.
- **Middle zone:** moderate sunlight, no vents. Resource-scarce. Predators and scavengers.
- **Bottom zone:** low/no sunlight, vent energy. Thermosynthetic specialists.

#### Decay Matter

Dead cells leave behind an energy deposit on their tile. Deposit energy starts at a fraction of the cell's energy at death. Decays over time according to configurable `decay_rate`. Scavengers extract energy via `scavenge_ability`.

Creates a nutrient cycle: predators kill, remains feed scavengers, scavengers die, new remains appear.

### Environmental Layers

#### Temperature Map

Generated at world init using Perlin noise (configurable frequency/scale). Each tile has a static temperature value. Cells pay extra metabolism when their `temperature_preference` gene mismatches local temperature.

Creates biome boundaries. Different specialists evolve in different thermal regions.

Optional future extension: slow temporal drift to simulate climate change, forcing migration and adaptation.

#### Toxin Map

Starts empty. Generated dynamically when multiple cells die in a localized area (death cluster). Cells without sufficient `toxin_resistance` take damage in toxic tiles. Creates wastelands that only resistant specialists can inhabit.

Toxin decays slowly over time.

#### Pheromone Map

Per-tile floating point value. Cells with `signal_emission` add to local pheromone each tick. Cells with `signal_sensitivity` read nearby pheromone concentrations to influence movement decisions.

Used for chemotaxis, swarm signaling, territory marking, and rally signals.

Decays relatively fast (signals are temporary). Diffuses to adjacent tiles each tick.

### Diffusion

All three continuous layers (pheromone, toxin, temperature) diffuse to neighboring tiles each tick using the same general formula:

```
new_value = value * (1 - decay_rate - spread_rate)
          + sum(neighbor_values) * (spread_rate / neighbor_count)
```

Each layer has independent tuning:

| Layer       | Decay     | Spread    | Neighbors | Behavior                                |
|-------------|-----------|-----------|-----------|----------------------------------------|
| Pheromone   | Fast      | Moderate  | 8 (Moore) | Temporary, local. Round plumes.         |
| Toxin       | Slow      | Slow      | 4 (Von Neumann) | Lingering, angular contamination zones. |
| Temperature | Near-zero | Very slow | 8 (Moore) | Mostly static. Enables future dynamic heat sources. |

Diffusion requires its own double buffer (two float grids, reused sequentially across layers). Processed during tick step 1 (decay phase) before cells sense anything.

### Tick Order

Each world step processes in this order:

1. **Decay phase** -- pheromone fades/diffuses, toxin fades/diffuses, temperature diffuses, decay matter fades
2. **Sensing phase** -- each cell reads local tile + neighbors within `sense_radius`, determines current phase state
3. **Decision phase** -- each cell selects an action (move, attack, reproduce, share energy, idle) based on genome, active phase modifiers, and sensed environment
4. **Action resolution** -- all actions resolved simultaneously from double-buffered state. Conflicts (two cells targeting same tile, mutual attacks) resolved by deterministic rules
5. **Energy update** -- photosynthesis/thermosynthesis income applied, metabolic drain applied, starvation deaths processed
6. **Cleanup** -- dead cells become decay matter, toxin generated at death clusters, pheromone contributions written

Simultaneous resolution ensures no cell has an advantage from processing order. Double-buffered grid: all cells read from "current" state, all writes go to "next" state, then swap.

### World Parameters (configurable at init)

| Parameter                       | Description                                           |
|---------------------------------|-------------------------------------------------------|
| `grid_width`, `grid_height`     | World dimensions                                      |
| `sunlight_gradient_strength`    | Steepness of top-to-bottom energy falloff              |
| `vent_count`                    | Number of thermal vents along bottom edge              |
| `vent_output`                   | Energy emitted per vent per tick                       |
| `vent_cycle`                    | Erupt/dormant period. 0 = always on                   |
| `decay_rate`                    | Speed at which remains lose energy                     |
| `pheromone_decay`               | Pheromone evaporation speed                            |
| `pheromone_diffusion`           | Pheromone spread rate to neighbors                     |
| `toxin_decay`                   | Toxin cleanup speed                                    |
| `toxin_generation_threshold`    | Deaths in area required to generate toxin              |
| `temperature_noise_scale`       | Perlin noise frequency for temperature map             |
| `initial_cell_count`            | Seed population size                                   |
| `initial_genome_strategy`       | Seeding method (see below)                             |
| `min_viable_acquisition`        | Floor value for highest energy acquisition gene at spawn. 0 = fully random. |
| `base_spawn_energy`             | Minimum starting energy for all spawned cells          |
| `bonus_spawn_energy`            | Additional energy scaled by genome viability score     |
| `cluster_count`                 | Number of clusters for random_clusters seeding strategy |

### Initial Seeding Strategies

| Strategy            | Description                                                                     |
|---------------------|---------------------------------------------------------------------------------|
| `random_uniform`    | Random genomes scattered uniformly. Chaotic start, slow convergence.            |
| `random_clusters`   | Random genomes placed in spatial clusters. Each cluster shares a common ancestor. Immediate local competition + divergence between clusters. |
| `preset_archetypes` | 3-4 hand-designed species (photosynthesizer, predator, scavenger, vent-feeder) with mutations. Controlled start. |

Recommended default: `random_clusters` for the most interesting early dynamics.

---

## Cell Spawning Mechanics

### Genome Initialization (Biased Random)

All 64 bytes are rolled uniformly random. After rolling, the four energy acquisition genes are checked: `photosynthesis_rate`, `thermosynthesis_rate`, `scavenge_ability`, and `predation_efficiency`. If none of these exceed the viability floor (`min_viable_acquisition`, default 40/255), the highest one is boosted to the floor.

This guarantees every cell has at least one working energy acquisition method without prescribing which one. The choice emerges from the random roll -- a cell might be a photosynthesizer, a scavenger, or a predator depending on which gene happened to be highest before the floor was applied.

Phase table bytes are left fully random. Most newly-spawned cells will have nonsensical phase triggers. That is intentional -- evolution cleans up the phase table over many generations.

Setting `min_viable_acquisition` to 0 disables the floor entirely, giving fully random behavior.

### Starting Energy (Scaled to Genome)

Starting energy is not fixed. It scales with how viable the genome actually is:

```
viability = max(effective_photosynthesis, effective_thermosynthesis,
                effective_scavenge, effective_predation)
            - effective_metabolic_cost

starting_energy = base_spawn_energy + (viability / max_viability) * bonus_spawn_energy
```

`base_spawn_energy` gives every cell a brief survival runway regardless of genome quality -- even a bad genome gets a few ticks to find food before starving. `bonus_spawn_energy` is the reward for efficiency: a well-built genome can start with 3-4x more energy than a poor one.

This creates soft selection at spawn. Bad genomes are not killed immediately, but they have a much shorter runway to find food or reproduce. Both parameters are configurable.

### Cluster Spawning (random_clusters Strategy)

When using `random_clusters`:

- The grid is divided into `cluster_count` evenly-spaced positions using a grid layout (not random placement) to guarantee clusters do not overlap at spawn.
- Each cluster position gets one **ancestor genome** generated via the biased random process above.
- All other members of the cluster are copies of the ancestor with light mutation applied, using the ancestor's own `mutation_rate` and `mutation_magnitude` genes.
- Cluster radius is `grid_width / (cluster_count * 2)`. Cells within a cluster are scattered randomly within this radius.

This means each cluster starts as a genetically similar population occupying the same neighborhood. The result is immediate intra-cluster competition (which variant of this lineage survives) and inter-cluster isolation (separate populations evolving independently until they expand and meet).

### Placement Rules

- No two cells may occupy the same tile at spawn.
- `random_clusters`: cluster centers are placed on a regular grid layout to guarantee spacing. Cells within each cluster are placed at random positions within the cluster radius.
- `random_uniform`: cells placed at random unique positions across the full grid.
- `preset_archetypes`: predefined cluster centers, one per archetype, equal population per cluster.

### Spawn Position and Zone Interaction

For `random_clusters`, cluster centers are distributed across the full Y axis. Some clusters land in high-sunlight zones near the top; some land near thermal vents at the bottom; some land in the resource-scarce middle.

The ancestor genome has no knowledge of where its cluster will spawn. A cluster with a high `photosynthesis_rate` ancestor that lands at the bottom will struggle -- there is no sunlight there. It will either die out or, if it survives long enough to reproduce, evolve toward thermosynthesis or predation as those genes become selectively advantageous.

This is intentional. Mismatched placement creates immediate directional selection pressure that drives early divergence between clusters.

---

## Planned Future Extensions

These are intentionally deferred from the initial implementation:

- **Continuous phase blending** -- phase modifiers interpolate smoothly based on condition intensity instead of discrete switching
- **Entropy-based specialization penalty** -- replace top-N gating with an entropy measure over gene distribution for smoother evolutionary gradients
- **Day/night cycle** -- sinusoidal multiplier on sunlight gradient
- **Seasons / climate drift** -- slow temperature map shifts over time
- **Obstacles / terrain** -- impassable tiles, walls
- **Water/land biomes** -- tile type distinction splitting the grid into fundamentally different environments
- **GPU compute shaders** -- move simulation to `wgpu` compute for grids beyond 1024x1024
- **Genome-driven action priority** -- replace hardcoded action priority (reproduce > attack > flee > move > idle) with genome-encoded action weights for probabilistic selection
- **Live parameter tuning UI** -- runtime sliders/controls to adjust world parameters (mutation rate, sunlight strength, etc.) without restarting
- **Dynamic vent heating** -- thermal vents warm surrounding tiles via temperature diffusion
- **Save/load simulation state** -- serialize full world state for replay, comparison, and branching experiments
- **Visualization overlays** -- toggleable heatmap renders for pheromone, toxin, energy, temperature, and genetic diversity
- **Population analytics** -- real-time graphs tracking population count, average energy, species count, gene distribution over time
