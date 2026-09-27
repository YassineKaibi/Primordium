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
| 2     | `predation_efficiency` | Percentage of victim's energy absorbed on kill, scaled within `max_predation_efficiency` (the gene has no antagonist). With `corpses_keep_energy`, the part the killer does not absorb stays on the victim's tile as decay matter. |
| 3     | `scavenge_ability`     | Energy extracted from decay matter on a tile, per tick, capped by `max_scavenge_per_tick`. It is a rate, not a swallow: uncapped, a maxed scavenger clears the whole tile on arrival and a corpse is one meal rather than a food source. |
| 4     | `energy_storage_cap`   | Maximum energy the cell can hold, between `energy_cap_floor` and `energy_cap_max`. Excess is wasted. |
| 5     | `base_metabolism`      | Passive energy drain per tick. Lower is more efficient, but antagonistic pairs and active gene expression push effective cost higher. |

#### Movement (6 genes)

| Index | Gene                  | Description                                                             |
|-------|-----------------------|-------------------------------------------------------------------------|
| 6     | `speed`               | Probability of moving each tick. 0 = sessile, 255 = moves every tick.   |
| 7     | `direction_bias`      | Preferred heading. 0-255 maps linearly to 0-360 degrees.               |
| 8     | `direction_noise`     | Randomness added to movement direction. Low = straight lines, high = Brownian. |
| 9     | `chemotaxis_strength` | Tendency to move toward nearby energy sources. With `foragers_stay_on_food` that includes the source underfoot: a cell's chance of moving is scaled by `1 - chemotaxis_strength * share`, where `share` is its own tile's food against the richest other food tile in sense range. |
| 10    | `flee_response`       | Tendency to move away from larger or aggressive neighbors. With `flee_can_escape`, a cell flees with probability `speed * flee_response`, only from the nearest non-kin whose blow (attack beats armour) or venom would cost it energy, and a strike misses a target that has moved out of the attacker's reach. |
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
| 17    | `reproduction_threshold` | Fraction of storage capacity at which the cell splits, never below `reproduction_energy_floor`. |
| 18    | `offspring_energy_share` | Percentage of parent energy transferred to offspring, bounded to `[min_offspring_energy_share, max_offspring_energy_share]` so that both leave the split alive: a share of 0 made a zero-energy child (a birth, a corpse, and `corpse_biomass` of decay from nothing) and a share of 1 killed the parent. |
| 19    | `mutation_rate`          | Per-gene probability of mutation during reproduction, scaled within `max_mutation_rate`. |
| 20    | `mutation_magnitude`     | Maximum shift applied to a mutated gene value, scaled within `max_mutation_magnitude`. |
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
| 30    | `decay_rate`| How long the cell's corpse persists as scavengeable decay matter. Scales the world's `decay_rate` between `corpse_decay_scale_min` and `corpse_decay_scale_max`, and travels with the deposit on the tile. |
| 31    | `membrane`  | Resistance to venom and environmental damage (toxin).                          |

#### Lifecycle (4 genes)

| Index | Gene               | Description                                                              |
|-------|--------------------|--------------------------------------------------------------------------|
| 32    | `max_age`          | Tick count before natural death, between `min_lifespan_ticks` and `max_lifespan_ticks`. |
| 33    | `maturity_age`     | Ticks before reproduction is unlocked. Capped by `max_age` (see Physical Caps). |
| 34    | `dormancy_trigger` | Energy threshold below which the cell enters dormancy phase, scaled into `[0, max_dormancy_trigger]` — dormancy is a last resort, not an operating mode. A dormant cell takes **no action at all** — it cannot reproduce, attack, flee or move until income lifts it back above the trigger. |
| 35    | `dormancy_cost`    | Energy drain rate while in dormancy (lower = better hibernation). Multiplies metabolic cost, floored at `min_dormancy_cost` so a cheap hibernator is not immortal; venom and toxin damage are unaffected. |

#### Environmental (3 genes)

| Index | Gene                   | Description                                                          |
|-------|------------------------|----------------------------------------------------------------------|
| 36    | `temperature_preference`| Optimal temperature zone. Mismatch with local temp = extra metabolism cost. |
| 37    | `toxin_resistance`     | Damage reduction from toxin exposure on polluted tiles.               |
| 38    | `adaptation_rate`      | Speed of within-lifetime epigenetic-like modifier shifts. Not inherited. Implemented as **thermal acclimation**: each tick a cell closes `adaptation_rate * max_adaptation_rate` of the gap between its effective temperature preference and its tile's temperature, and pays its mismatch cost against that acclimated preference. The shift lives on the cell and starts at zero in every newborn. |

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
| 43    | `gene_linkage`        | Controls which gene clusters tend to mutate together (simulates chromosomes). It redistributes the mutational load rather than adding to it: a linked block runs `1 / (1 - linkage)` bytes on average and the chance of starting one is divided by the same factor. |
| 44    | `horizontal_transfer` | Probability of absorbing genes from consumed cells into own genome.     |
| 45    | `transposon_rate`     | Rate of internal gene duplication and shuffling within the genome.      |

---

### Phase Table (18 bytes)

3 phase slots, 6 bytes each. Phases modify gene expression based on environmental conditions without changing the genome itself.

**Phase resolution order:** slots are evaluated 0, 1, 2. First match wins. No match = default active state (no modifiers applied).

Evolution can disable a slot by setting `trigger_threshold` to its maximum: a threshold of 0 fires always, 255 effectively never.

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

Entry happens when the condition rises to the threshold. Exit requires it to fall a band *below* the threshold (`threshold * (1 - band)`), so a phase is sticky once entered. Every trigger condition is normalised so that higher = stronger, so a slot is effectively disabled by setting `trigger_threshold` above what the condition can ever reach, not by polarity.

---

### Expression Constraints

Three mechanisms prevent convergence toward homogeneous "supercells."

They are applied in this order, and the order is load-bearing:

```
normalize -> top-N gating -> antagonistic pairs -> physical caps
```

**Gating decides what the cell expresses at all; the pairs then trade off between
the things it does express.** Running the pairs first couples two mechanisms that
are supposed to be independent: a gene is cut by its partners, and then cut *again*
by the falloff, because those cuts cost it its rank. `speed` sits in three pairs,
more than any other gene, and under the old order it was gated in **99.8% of random
genomes** — which closed both mobile niches, scavenging and predation, to every
genome the world could roll. Only sessile strategies survived.

It also means a gene the cell is not expressing exerts no antagonistic pressure,
which is the physically coherent reading: armour a cell is not growing should not be
slowing it down.

#### 1. Metabolic Budget

Every gene has an expression cost. Total expression cost = effective per-tick energy drain, scaled by `metabolic_cost_scale`. Costs scale **superlinearly** (exponent 1.5-2.0): pushing a gene from 50% to 100% effectiveness costs disproportionately more than 0% to 50%.

The scale sets the absolute level of the whole economy, and the invariant below is what it must preserve: at the default (0.2 x 46 genes = 9.2 for a maxed genome) an all-max cell still drains far faster than `photo_max_income` (4.0), while a lean specialist pays well under 1.0.

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
- `maturity_age` capped by `max_age` (`maturity_lifespan_fraction` of the cell's own lifespan) — a cell that matures after it dies is sterile by construction, and a whole founder lineage can be lost that way

---

### Visual Representation

Cell color is a continuous function of what the cell is, so genetically similar cells appear visually similar and speciation shows as color clustering:

- **Hue band** from the dominant acquisition strategy (photosynthesis green, thermosynthesis ember, scavenging ochre, predation magenta, none slate).
- **Hue within the band** (±22°) from the genome hash, so a lineage drifts gradually instead of jumping. A one-byte mutation moves hue ~15°; unrelated genomes sit ~77° apart.
- **Saturation** from how specialized the cell is (the gap between its best acquisition gene and the runner-up).
- **Brightness** from energy as a share of the cell's own storage cap, so starvation is visible.

A plain hash-to-HSV mapping does *not* satisfy this: it moved hue ~88° for a single changed byte, which is indistinguishable from an unrelated genome.

The renderer also offers debug modes that replace this mapping: flat color per strategy, flat color per active phase, and an energy heat ramp.

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
| `sunlight`     | u8   | Dynamic. Recomputed each tick via Beer-Lambert column scan. |

Approximate memory: ~18 bytes/tile. At 1024x1024 = ~18MB per buffer, ~36MB with double buffering.

### Energy Sources

Three channels for energy entering the system:

#### Sunlight

Light enters from the top (y=0) and attenuates with depth following **Beer-Lambert law**: `I = I₀ · e^(-α · depth)`. Each tile has a per-tile absorption coefficient α composed of:

- **Base water absorption** — clear water still absorbs some light (`sunlight_gradient_strength / grid_height` per row)
- **Cell presence** — living cells occupying a tile block light (+0.2 α)
- **Decay matter** — dead cell remains cloud the water (`decay_energy * 0.004`)
- **Toxin** — pollution darkens the water (`toxin * 0.002`)
- **Pheromone** — chemical signals add slight murkiness (`pheromone * 0.001`)

Sunlight is recomputed each tick via a top-to-bottom column scan after diffusion, before sensing. This means a dense colony of photosynthesizers near the surface will shade out cells below them — creating emergent competition for light access.

Top rows receive maximum energy, bottom rows receive minimal or zero. Photosynthesizers absorb energy proportional to `photosynthesis_rate` and local sunlight intensity.

Creates a "surface" zone where photosynthetic cells thrive.

**Light is the one field that does not wrap.** The grid is toroidal on both axes (see Grid Properties), but each column's scan restarts at full intensity at `y = 0`, so the last row and the first row are neighbours in space and maximum-distance apart in light. That is intended, and it is what makes the vertical resource axis below a real axis rather than a ring: without it there would be no "top". The consequence is that a cell can step from the darkest row into the brightest one, and that the two producer niches — photic and thermal — border each other at the seam. Anything else that reasons about depth must therefore measure `y` from the bottom row *without* wrapping, the way `World::is_in_vent_zone` does; wrapping it put vent energy into the photic rows.

#### Thermal Vents

Localized high-energy sources positioned along the **bottom edge** of the grid. Fixed positions, finite output per tick shared among every cell within `vent_radius` tiles. The radius is what makes the bottom zone a zone: at a radius of 1 a vent feeds nine tiles out of a quarter-million, and no founder cluster ever reached one.

Can be configured as:
- **Permanent:** constant output
- **Periodic:** erupt for N ticks, dormant for M ticks

Cells absorb vent energy proportional to `thermosynthesis_rate`. This creates a distinct bottom-dwelling ecological niche, separate from photosynthesizers.

The vertical resource axis:
- **Top zone:** high sunlight, no vents. Photosynthesizers dominate.
- **Middle zone:** moderate sunlight, no vents. Resource-scarce. Predators and scavengers.
- **Bottom zone:** low/no sunlight, vent energy. Thermosynthetic specialists.

#### Decay Matter

Dead cells leave behind an energy deposit on their tile: `corpse_biomass` of structural matter plus `corpse_energy_fraction` of whatever energy remained. The structural part matters — a starved cell has no energy left but still has a body, and without it starvation deaths fed no one. With `corpses_keep_energy` this holds for every corpse: a cell that dies of old age leaves `corpse_energy_fraction` of the energy it still held (without the switch senescence zeroes that energy first, and an old corpse leaves only the biomass), and a killed cell's tile also receives whatever part of its pre-blow energy its killer did not absorb. Decays over time according to configurable `decay_rate`. Scavengers extract energy via `scavenge_ability`.

Creates a nutrient cycle: predators kill, remains feed scavengers, scavengers die, new remains appear.

The cycle has to be started. Nothing dies for the first several hundred ticks of a run -- `max_age` maps to a 300-3000 tick lifespan -- while a scavenger's runway is its storage cap over its upkeep, about 157 ticks. Scavengers seeded at tick 0 therefore starve before their food exists, measured with `decay_current` at 0.0 for the first ~70 ticks. `initial_decay_matter` puts pre-existing detritus on every tile at world creation so the niche exists from the start.

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

After diffusion, sunlight is recomputed via a Beer-Lambert column scan (see Sunlight section above). This must happen after diffusion because the per-tile absorption coefficient depends on current toxin, pheromone, and decay levels.

### Tick Order

Each world step processes in this order:

1. **Decay phase** -- pheromone fades/diffuses, toxin fades/diffuses, temperature diffuses, decay matter fades, sunlight recomputed (Beer-Lambert column scan)
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
| `vent_radius`                   | How far a vent's output reaches, in tiles              |
| `vent_cycle`                    | Erupt/dormant period. 0 = always on                   |
| `decay_rate`                    | Speed at which remains lose energy                     |
| `initial_decay_matter`          | Detritus every tile starts with, so scavengers have food at tick 0 |
| `max_scavenge_per_tick`         | Cap on decay one cell can strip from a tile in one tick |
| `max_dormancy_trigger`          | Highest energy fraction a `dormancy_trigger` of 255 can mean |
| `min_dormancy_cost`             | Floor on the metabolic discount `dormancy_cost` can buy |
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
| `archetype_band_depth`          | Rows per preset_archetypes band; 0 derives it from the band's population |
| `archetype_population_shares`   | How initial_cell_count splits between the four archetypes |
| `metabolic_cost_scale`          | Multiplier on the summed per-gene expression cost      |
| `photo_max_income`              | Income of a perfect photosynthesizer in full sunlight   |
| `scavenge_efficiency`           | Fraction of consumed decay matter that becomes energy   |
| `energy_cap_floor`              | Storage capacity when `energy_storage_cap` is 0        |
| `energy_cap_max`                | Storage capacity when `energy_storage_cap` is 255      |
| `reproduction_energy_floor`     | Absolute energy required to split, whatever the cap    |
| `min_offspring_energy_share`    | Least share of parent energy a child is born with       |
| `max_offspring_energy_share`    | Most share of its energy a parent can give its child    |
| `max_mutation_rate`             | Per-byte mutation chance when `mutation_rate` is 255    |
| `max_mutation_magnitude`        | Largest byte shift when `mutation_magnitude` is 255     |
| `max_adaptation_rate`           | Share of the temperature gap closed per tick at `adaptation_rate` 255 |
| `max_move_distance`             | Furthest a move goes in one tick; 1 is one tile per tick |
| `attack_only_when_harmful`      | Attack only a threat whose armour the blow can beat; otherwise flee |
| `flee_can_escape`               | Flee takes a speed roll, only from threats that can hurt, and movement resolves before blows land |
| `food_targets_richest`          | Steer toward the richest food in range rather than the nearest |
| `foragers_stay_on_food`         | A cell on food moves less, by its chemotaxis times its own tile's share of the food around |
| `max_transposon_rate`           | Chance per birth of a transposon event at `transposon_rate` 255 |
| `max_horizontal_transfer`       | Chance per kill of absorbing a victim gene at `horizontal_transfer` 255 |
| `corpse_decay_scale_min`        | Multiplier on `decay_rate` for a corpse whose `decay_rate` gene is 0 |
| `corpse_decay_scale_max`        | Multiplier on `decay_rate` for a corpse whose `decay_rate` gene is 255 |
| `min_lifespan_ticks`            | Lifespan when `max_age` is 0                            |
| `max_lifespan_ticks`            | Lifespan when `max_age` is 255                          |
| `max_maturity_ticks`            | Maturity delay when `maturity_age` is 255               |
| `maturity_lifespan_fraction`    | Cap on maturity as a share of the cell's own lifespan   |
| `corpse_biomass`                | Structural decay matter every corpse leaves behind      |
| `corpse_energy_fraction`        | Share of a corpse's remaining energy that becomes decay |
| `corpses_keep_energy`           | An old corpse keeps its energy, and a kill's uneaten part stays on the victim's tile |
| `max_predation_efficiency`      | Share of a victim's energy a kill pays at `predation_efficiency` 255 |

### Initial Seeding Strategies

| Strategy            | Description                                                                     |
|---------------------|---------------------------------------------------------------------------------|
| `random_uniform`    | Random genomes scattered uniformly. Chaotic start, slow convergence.            |
| `random_clusters`   | Random genomes placed in spatial clusters. Each cluster shares a common ancestor. Immediate local competition + divergence between clusters. |
| `preset_archetypes` | 4 hand-designed species (photosynthesizer, vent-feeder, scavenger, predator) with mutations. Controlled start: the trophic links are already in contact, so this measures whether a food web is *sustainable* here, separately from whether random founders ever assemble one. |

Recommended default: `random_clusters` for the most interesting early dynamics.

---

## Cell Spawning Mechanics

### Genome Initialization (Biased Random)

All 64 bytes are rolled uniformly random. After rolling, the four energy acquisition genes are checked: `photosynthesis_rate`, `thermosynthesis_rate`, `scavenge_ability`, and `predation_efficiency`. If none of them **expresses** above the viability floor (`min_viable_acquisition`, default 40/255), the genome is adjusted until one does.

The test is on the decoded value, not the raw byte. Checking the byte does not guarantee anything: top-N gating attenuates every gene outside a genome's dozen strongest, so an acquisition gene sitting exactly at the floor is routinely outranked by random parameter genes and decodes ten times smaller. Measured on the byte-only check, **41.7% of founders decoded below the floor** the check was supposed to guarantee.

The adjustment tries each acquisition gene in descending raw order and, for each:

- cuts the other three back to the floor, because `photosynthesis_rate` and `thermosynthesis_rate` are themselves an antagonistic pair — a roll that is high in both expresses neither (photo 255 with thermo 183 decodes to 0.392 before gating and 0.039 after);
- raises the candidate until it decodes above the floor *and* ranks inside the top N.

The first candidate that works is kept. Antagonistic pairs still cut the gene down afterwards, and they should: a genome that rolled a high `speed` cannot be a photosynthesizer, because the two are a pair — but it makes a perfectly good hunter, and that is which gene the search settles on.

This guarantees every cell has at least one working energy acquisition method without prescribing which one. The choice emerges from the random roll -- a cell might be a photosynthesizer, a scavenger, or a predator depending on which gene happened to be highest, and on which one the rest of its genome will let it express.

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

Starting energy is clamped to the genome's own storage capacity: anything above the cap would be wasted on the first tick anyway.

`base_spawn_energy` gives every cell a brief survival runway regardless of genome quality -- even a bad genome gets a few ticks to find food before starving. `bonus_spawn_energy` is the reward for efficiency: a well-built genome can start with 3-4x more energy than a poor one.

This creates soft selection at spawn. Bad genomes are not killed immediately, but they have a much shorter runway to find food or reproduce. Both parameters are configurable.

### Cluster Spawning (random_clusters Strategy)

When using `random_clusters`:

- The grid is divided into `cluster_count` evenly-spaced positions using a grid layout (not random placement) to guarantee clusters do not overlap at spawn. Rows span the **full** Y axis, top and bottom edges included, so that some clusters really do start in the vent zone; centring each row inside its own band instead leaves every founder tens of tiles short of the bottom.
- Each cluster position gets one **ancestor genome** generated via the biased random process above.
- All other members of the cluster are copies of the ancestor with light mutation applied, using the ancestor's own `mutation_rate` and `mutation_magnitude` genes.
- Cluster radius is `grid_width / (cluster_count * 2)`. Cells within a cluster are scattered randomly within this radius.

This means each cluster starts as a genetically similar population occupying the same neighborhood. The result is immediate intra-cluster competition (which variant of this lineage survives) and inter-cluster isolation (separate populations evolving independently until they expand and meet).

### Placement Rules

- No two cells may occupy the same tile at spawn.
- `random_clusters`: cluster centers are placed on a regular grid layout to guarantee spacing. Cells within each cluster are placed at random positions within the cluster radius.
- `random_uniform`: cells placed at random unique positions across the full grid.
- `preset_archetypes`: one full-width band per archetype, stacked in contact (see below). Populations are split by `archetype_population_shares`, equal by default.

### Archetype Bands (preset_archetypes Strategy)

The four archetypes are hand-built rather than rolled: each carries the genes its
strategy needs and leaves everything else near zero, so it clears its own upkeep in
its own niche on tick 1. Phase tables are switched off (a `trigger_threshold` above
what the condition can reach) except for one designed slot on the predator; the
random strategies' founder phase noise is deliberate, and a controlled start should
not also be measuring it. Mutation reopens the slots over a run.

**The four designs, and why each is shaped the way it is** (the measurements behind
each choice are in `docs/handover/HANDOVER.md`, step 12):

- **Photosynthesizer and vent-feeder** -- sessile primary producers carrying enough
  `armor` that a *sated* predator's blow does no damage.
- **Scavenger** -- a sessile **mixotroph**: scavenging is its strongest channel, with
  photosynthesis as a second income and a low reproduction threshold. A mobile, pure
  detritivore cannot pay for its own search in this world (sense, chemotaxis and
  speed are all expression cost, and corpses are intermittent); the scavengers that
  evolve under random seeding are nearly sessile and photosynthesize too. Being
  sessile, it pays nothing for armour (`armor <-> speed` only bites a mover).
- **Predator** -- sustainable rather than a plague because of three brakes:
  - *hunger*: its base `attack_power` does no damage through prey armour, and one
    phase slot (EnergyLow, firing below ~25% of its cap) doubles its offense, so it
    kills only when it needs to eat;
  - *maturity*: a newborn cannot breed at once, otherwise every newborn goes
    kill -> breed -> kill and breeding chains double every few ticks;
  - *cooldown*: a mature predator breeds at most once per ~100 ticks, otherwise a
    cohort that matures together breeds repeatedly at once.

**`max_maturity_ticks` must be around 300 for this strategy.** Maturity is the brake
that keeps newborn predators from breeding at once, and at a cap of 30 no gene can
express it. The random strategies do as well or better at 30, so the default is not
changed; `docs/handover/archetypes.json` is a ready-made config for this strategy.

Each archetype gets a **full-width horizontal band**, and the bands stack around the
`y` wrap seam, which is where this world's two energy sources meet: light enters at
`y = 0` and is absorbed downward, while the vents sit on the bottom row.

```text
  y = 0            photosynthesizer   brightest rows, nothing above to shade them
    (same rows)    scavenger          interleaved: a mixotroph needs the light, and
                                      producers die where it stands
  below it         predator           in contact with the producers it eats
  ...
  y = H - depth    vent-feeder        the vent row, adjacent to the photic band
```

Bands rather than square clusters, because of the light column. Every occupied tile
absorbs 0.2 of the light passing through it, so a producer colony that is deep in `y`
shades itself out: a 51x51 square block of founders measured a mean sunlight of 15.6
of 255 on its own tiles. The same effect is why the scavenger shares the photic rows:
in its own band beneath the photosynthesizers it earned 0.17 a tick against an upkeep
of 0.88. Band depth is derived from each archetype's founder count (about half the
tiles filled) and can be overridden with `archetype_band_depth`.

`archetype_population_shares` splits `initial_cell_count` between the four, in the
order photosynthesizer, vent-feeder, scavenger, predator. Equal shares are the
default.

**What this start achieves** (measured, `archetypes.json`): all four seeded lineages
coexist to t = 4 000 on every seed tried, under both equal and pyramid shares, and to
t = 8 000 on two seeds of three. The failure mode is evolutionary: over thousands of
ticks predator numbers creep up, consistent with selection moving their hunger
trigger toward greed, until they overshoot their prey.

### Spawn Position and Zone Interaction

For `random_clusters`, cluster centers are distributed across the full Y axis. Some clusters land in high-sunlight zones near the top; some land near thermal vents at the bottom; some land in the resource-scarce middle.

The ancestor genome has no knowledge of where its cluster will spawn. A cluster with a high `photosynthesis_rate` ancestor that lands at the bottom will struggle -- there is no sunlight there. It will either die out or, if it survives long enough to reproduce, evolve toward thermosynthesis or predation as those genes become selectively advantageous.

This is intentional. Mismatched placement creates immediate directional selection pressure that drives early divergence between clusters.

---

## Planned Future Extensions

These are intentionally deferred from the initial implementation:

- **Wider within-lifetime adaptation** -- `adaptation_rate` (gene 38) currently drives thermal acclimation only; other traits with a clear local target could shift the same way.
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
