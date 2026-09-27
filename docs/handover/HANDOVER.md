# Handover: species balance and phase visibility

## Start here (2026-09-27, updated after step 16)

**Problem:** the world can't sustain more than one way of living — photosynthesizers
take over, scavengers and predators die — and the window shows life only in a thin
strip. Steps 1–16 below record every fix with its before/after measurement.

**Step 15 changed how things are measured — read it first.** The harness is no longer
a patch: it is `src/bin/lab` (`cargo run --release --bin lab`), reading counters the
sim keeps itself (`sim/stats.rs`). Instrumenting the sim changed no behaviour (state
hashes bit-identical on 3 configs x 2 seeds). The success criterion is no longer "who
is most numerous" but "does every way of living persist", scored over a window on
10 paired seeds (`--score`), plus the grow-from-rare test (`--invade`).

**What step 15 measured** (full tables there):
1. Baseline, your defaults (10 seeds x 2000 ticks): photo and scav persist on 10/10,
   thermo 5/10, **hunters (by what they eat) 0/10**; life sits in 10–35 rows.
2. `archetypes.json`: all four groups persist on 7/10 to t=2000; life in ~9 rows.
3. **The designed predator is not self-sustaining.** With mutation off it declines
   1250 → 5 by t=2000; introduced at t=1000 into producers + scavengers it shrinks
   on 6/6 seeds (r = −0.0015 ± 0.0001 per capita-tick). With mutation on, the trait
   that drifts is **maturity** (200 → ~110 ticks on seed 3), not the hunger threshold
   or attack. "Predators evolve greedier" is: a declining predator is selected to
   breed sooner, which is the brake step 12 found stops breeding chains.
4. **The archetype "scavenger" mostly photosynthesizes**: ~1 490 gene-scavengers, ~190
   living mainly on decay. Introduced at t=1000 all starve (6/6 seeds, 85% of their
   income was light). A sessile cell can only eat decay on its own tile and decay
   never moves, so the scavenger niche is the t=0 detritus windfall.
5. **Stillbirth bug (fixed in step 16):** an `offspring_energy_share` that decodes to 0 makes
   zero-energy children that count as births, die at once and leave 25 decay each —
   energy from nothing. Parents with a share of 1.0 die giving birth (~2% of deaths).

**What step 16 changed and measured** (full tables there): four mechanism changes,
each judged by whether a consumer's grow-from-rare rate flips. **None flips.**
1. Stillbirth bug **fixed**: the offspring share is bounded to [0.05, 0.95]. Stillborn
   and childbirth deaths 940 + 72 412 → 0 + 0 over 10 seeds, births dead in their
   birth tick 1.15% → 0.03%. This is the new baseline.
2. Capping trophic transfer makes the predator **worse** (r −0.0013 → −0.0027 to −0.0032)
   and crashes the archetype web: hungrier predators kill twice as often. Kept corpse
   energy (spec-conformant old-age corpses, uneaten kill energy) adds scavengers.
3. Real Flee changes nothing measurable: prey are sessile and flight is ~0% of
   actions. The predator's failure is demographic: 70% of invaders die of old age;
   a gated `max_age` gives it 401 ticks, maturity at 200, a birth every 100.
4. Staying on food adds default-world scavengers (+404 ± 190). The standard scavenger
   invasion cannot be reached by #2 or #4 (sessile invader, dead before the first
   old-age corpses at t≈1316). A **mobile** scavenger grows from rare (r = +0.44) and
   then crashes on its own boom; with kept energy + staying, 3/6 seeds persist.
5. Proposed default flips (not made): `corpses_keep_energy`, `foragers_stay_on_food`.

**Remains:**
0. After step 16: measure the predator's reproduction brake (gated `max_age` →
   401-tick life, maturity capped at half of it, cooldown 100) against its energy;
   decide whether the scavenger assay should use a mobile invader or a later `--at`
   (after the old-age wave), since the sessile one at t=1000 cannot see corpse
   changes at all; fix `attack_only_when_harmful` ignoring venom before judging it.
1. The predator has no niche from rare (item 3). Step 16 tried two of the approach
   review's levers, capping trophic transfer and making Flee real: the cap makes it
   worse and Flee changes nothing. The third, what keeps maturity from sliding, is
   now item 0.
2. Scavengers need decay they can reach (item 4). Step 16 built staying on food and
   routing a kill's uneaten energy (and old corpses' energy) to decay; letting decay
   move (sink) is untried.
3. Life confined to ~9–35 rows: each occupied tile absorbs 0.2 of the light below it
   (hard-coded in `world::tile_absorption`, not yet in config).
4. Movement switches: `max_move_distance` and `food_targets_richest` still
   unmeasured; `attack_only_when_harmful` measured in step 16 (archetype hunters
   +36 ± 21), but it ignores venom and sense radius never exceeds 3 — fix first.

**User's defaults** (their edit to `config.rs`): `random_uniform`, 1 000 cells,
`max_maturity_ticks: 30`. Archetypes: `docs/handover/archetypes.json`.
**Rules:** never commit; measure before claiming a cause; keep harness runs short
(≤2000 ticks, ≤3 jobs); every fix gets a test that fails on the old code; a new
knob defaults to the old behaviour and is checked bit-identical with `lab --hash`.

Written 2026-09-19 for the next session. The goal is to keep finding and then fixing the reasons Primordium has no stable mix of strategies ("usually photosynthesizers remain and the rest die") and why evolved phases are not visible.

## 1. Repo state (read first)

- **Branch `salvage/recovery`**, fast-forwarded to `775b923` (GitHub `main`, PR #10) on 2026-09-20. Nothing is committed. The user handles all git operations. A PreToolUse hook blocks Write/Edit while on `main`/`master`, so stay on this branch.
- **Uncommitted changes:**
  - `@veridikt` comment annotations across `src/` (comments only; the user says code matters more than annotations).
  - Step-1 fixes (see section 4): `sim/world.rs`, `sim/tick.rs`, `sim/actions.rs`.
  - `src/render/` is no longer a local copy — the pull landed it as `775b923`. A leftover `stash@{0}` ("local render copies") is redundant and safe to drop.
  - This `docs/handover/` directory. `lab-harness.patch` in it is obsolete since step 15 (the lab is `src/bin/lab`) and can be deleted.
  - Step 15: `src/lib.rs`, `src/bin/lab/`, `src/sim/stats.rs` are new files; commit them together with `Cargo.toml` (`default-run`).
- `cargo test` passes (261 library + 8 lab tests as of step 16). `cargo clippy --all-targets -- -D warnings` and `cargo fmt -- --check` pass — the `empty_line_after_doc_comments` lint is `allow`ed at both crate roots (`main.rs`, `lib.rs`) because it fires on the `@veridikt` convention CLAUDE.md mandates. `#![allow(dead_code)]` is gone from `sim/mod.rs`: with a library crate nothing public is dead.
- **What was lost:** the user's April 11–17 work was never pushed: branch 9 `feat/main-loop` (winit + pixels window, sim thread), debug color mode, fast-forward, decay/vent seeding, predator hunger gating and other balance tweaks, plus `docs/plan.md`. The original directory was deleted on 2026-06-10 and re-cloned from a stale mirror (`~/.gitnexus/repos/Primordium`, taken 2026-04-11 00:55).
- **Where I looked for it:** reflog and dangling objects, the GitHub API (all PR refs, events, activity; `feat/main-loop` was never pushed), gists and repos, Claude transcripts, paste-cache and file-history (April purged by 30-day cleanup), micro backups, VS Code history, Trash, and a disk-wide grep. Only PR #10 was recoverable.
- **What survives of April:** `april-prompts.md` in this directory is the user's prompt log from the lost sessions. It is a symptom log of what the running app showed.
- **Consequence (no longer true as of 2026-09-21):** the window has been rebuilt — see "The window runs again". Observations below still come from the headless harness, which remains the measurement tool.

## 2. Harness (reproduce everything here)

**As of step 15 the harness is `src/bin/lab`, built from the repo.** The old
`lab-harness.patch` (and `sync-lab*.sh`) is obsolete: it no longer applies, its
instruments now live in the sim (`sim/stats.rs`), and every instruction below that
says "regenerate the patch" is history. Section 2's old flag/field tables are
replaced by this one.

```bash
cargo build --release --bin lab
L=./target/release/lab
$L --config docs/handover/archetypes.json --ticks 500 --every 100 | python3 docs/handover/summarize.py
$L --score --seeds 1-10 --ticks 2000                       # scorecard, your defaults
$L --score --seeds 1-10 --vs-set max_move_distance=3       # paired A/B on the same seeds
$L --invade pred --config docs/handover/archetypes.json \
   --set 'archetype_population_shares=[0.6,0.2,0.2,0]' --at 1000 --ticks 2000 --seeds 1-6
```

Config: `--config file.json` (missing fields take defaults, unknown fields are an
error), then `--seed --w --h --cells --clusters --uniform`, then `--set key=value`
(any `WorldConfig` field; JSON values, or bare strings for enums). Unknown flags are
errors. Without `--config` the lab runs `WorldConfig::default()` (the old lab ran
256², 2000 cells).

| Mode | What it prints |
|---|---|
| (default) run | One JSON line per `--every` ticks: `class` (by diet), `gene_class`, `rows_90`, `effective_groups`, `per_class` (lifetime income/upkeep per tick of age, repro-ready share), `lineages`/`top_lin`/`xtab`, `phase`, `per_tick` (births, deaths, `death_causes`, kills, attacks), `actions_pct`, `energy_per_cell_tick` (incl. predation), `phase_pct`/`ms_per_tick`, `genetics`, `repro_gate` (incl. `blocked_dormant`), `vents`, `integrity`. `--traits`: per lineage, attack/armour as applied, hunger slot, maturity, cooldown, and `bite_margin`. |
| `--score` | Per seed and arm: per group min/mean/CV/log-slope/persisted/established/extinct_at, `groups_persisting`, `first_loss`, `effective_groups`, `rows_90`, `non_default_phase`; a summary per arm and, with `--vs-set`/`--vs`, paired B−A differences with standard errors. `--series f.jsonl` writes the full time series. |
| `--invade X` | Growth from rare of archetype X (or a 128-hex genome) introduced at `--at` into its own band: per-capita births/deaths while rare (until `--rare-cap`, default 5n), `reached_rare_cap`, extinct_at, invader causes of death and what the dead lived on, residents before/after (excluding invaders). |
| `--hash` | Per seed, a hash of the full world state (every live cell's fields and record, every tile field) every `--every` ticks (`--seeds 1-2 --ticks 600 --every 100`). Two runs printing the same lines ran bit-identically: the check that a knob at its default changes nothing (step 16). |
| `--archetypes` `--niche` `--color-check` `--render` `--dump-config` | As before. |

**Classes are by diet.** A cell's class is the channel that has paid it most over its
life (`CellRecord::diet`), once that channel has paid ≥10% of its lifetime upkeep;
before that, its genes (`world::strategy_of`). A group counts as *established* only
once ≥`--n-min` cells actually live on it, so gene-classed founders at t=0 never count
as a group that was "lost". Old reports classified by genes with a 0.35 attack cutoff;
`gene_class` keeps that view for comparison.

**Paired arms** run the same seeds; when the change leaves seeding alone both arms
start from identical founders, so the founder lottery cancels out of the difference.

## 3. Baseline measurement (salvaged code, `default.json`)

- Seeds 1–3 go 4992 → 0 by tick 25–30. Nobody survives long enough for selection, reproduction or phases to matter.
- Income is photo 0.000, thermo 0.000, scav 0.000 per cell-tick; metabolism is about 9.
- With the prepare_next fix, all runs are still extinct by tick 50; photo income 0.02–0.09 vs metabolism about 9.
- In the first 10 ticks, about 620–710 attacks per tick. Actions at tick ≤5: Attack 52–61%, Share 20–29%, Idle 13–15%, Move 0.4%.

The April prompts show the user's lost version patched some of these symptoms ad hoc. The root causes are all still here.

## 4. Verified issues

Status key:

- **CONFIRMED** — code citation plus a harness or computed measurement.
- **CONFIRMED-static** — unambiguous from the code, not separately measured.
- **PARTIAL** — the mechanism is real, but the impact differs from the original claim.
- **FIXED** — repaired in the working tree, with a unit test that fails on the old code and a before/after harness measurement.

All line numbers refer to the salvaged tree.

| ID | Sev | Status | Issue | Evidence | Measured | Explains April symptom |
|---|---|---|---|---|---|---|
| B1 | critical | **FIXED** | Env fields lost across the buffer swap. `clear_next` resets next to `Tile::EMPTY`, keeping only temperature, and nothing copies sunlight/decay/pheromone/toxin current→next. The energy step reads the **next** grid, so photosynthesis and scavenging pay 0 and toxin never hurts. Accumulated pheromone/toxin/decay survive only what cleanup re-deposits that same tick. | `world.rs:262-267`, `tick.rs:191,198` | Before: `sun_used` 0.0 vs `sun_current` 54.5; decay at energy time 0.000. After: `sun_used == sun_current` (42.7 / 39.9 / 43.0 on seeds 1–3 at t=10) and `decay_used == decay_current` | "died in milliseconds", "decay fades way too quickly", "decay not rendered" |
| B5 | critical | **FIXED** | The energy budget is impossible. `metabolic_cost` sums `v^1.5` over all 46 decoded genes, including parameter genes and ~15 no-op genes. The best possible income is `PHOTO_MAX_INCOME` = 4.0 (perfect specialist in full sun). | `energy.rs:19,120-123` | Before: metab ≈9/cell-tick, extinct by t=50. After: metab ≈1.9, populations **grow** to t=1000 | "all die eventually" after each April fix |
| B2 | critical | **FIXED** | `Cell.position` is never updated after Move/Flee. `place_cell` only writes `tile.cell_id`, and there is no assignment to `.position` outside `Cell::new`. Next tick the cell senses and computes moves from its stale spawn tile, and Idle/Attack/Share/Reproduce re-place it there (teleport back, possibly overwriting another cell). | `actions.rs:688`, grep `position =` | Before: `pos_mismatch` up to 9 by t=5. After: 0 on every seed and tick measured | "no cell moved", "~1% move a few pixels" |
| B3 | critical | **FIXED** | Predation gives no energy. `PREDATION_EFFICIENCY` is read only by the spawner's viability floor. Attacks subtract damage from both sides and transfer nothing, even on a kill, so attacking is pure loss. | `actions.rs:593`, `:748-772`; `spawner.rs:26` | — | "predators never eat, vanish instantly" |
| B12 | high | **FIXED** | Kin recognition fires on siblings. `effective_trigger = aggression * (0.5+0.5*precision)` uses decoded genes, usually top-N-gated ×0.1, so near-identical cluster-mates count as "threats". The Attack gate precedes Move, so cells attack siblings regardless of their own attack power. | `actions.rs:324,355` | Before: **100% of attacks same-lineage**, 620–710 attacks/tick. After: same-lineage fraction **0.0**, 0.0–0.1 attacks/tick | Mass die-off in the first 10 ticks |
| B15 | high | **FIXED** | Almost no heredity. `mutate` hits each of the 64 bytes with p = rate/255, shifting by up to `magnitude` (both random 0–255 in founders), including the mutation genes and phase bytes. | `genome.rs:192-202` | Before: child differs in **35–58 / 64 bytes**, parent→child distance 0.07–0.36 (strangers ≈0.33). After: **1.0–1.4 bytes**, distance 0.0003–0.0006 | Selection cannot accumulate anything, including phases |
| B6 | high | **FIXED** | Storage-cap clipping. Spawn energy (50 + up to 150) far exceeds `ENERGY_STORAGE_CAP*255` for most founders and is destroyed on tick 1. The reproduction threshold is `gene * cap`, so a tiny cap also makes reproduction trivially cheap. | `spawner.rs:79-97`, `energy.rs:216`, `actions.rs` `mapped_reproduction_threshold` | Before: mean cap 15–65 vs spawn 108–120, `cap_waste` ≈36/cell-tick. After: spawn clamped to cap, `cap_waste` 0.05–0.27 | "one cluster reproduces all at once before vanishing" |
| B7 | high | **FIXED** | Phase modifiers never reach action resolution. `resolve_all` re-decodes genomes without `apply_phase_modifiers` for combat (attack/armor/venom), movement conflicts (rigidity), reproduction (offspring share, cooldown) and share. Vent income also decodes without modifiers, while photo/scav use modified genes. The offense and defense phase groups have **no effect**; only mobility (via `decide`) and efficiency (via the energy step) do anything. | `actions.rs:727,748,814,836`; `tick.rs:162` | — | "phases aren't visible, if they even evolved" |
| P1 | high | **RESOLVED — working as specified** | Phase occupancy is noise. Random founder phase tables put most cells in a non-default phase immediately: threshold byte < 4 ⇒ always on; AgeMature saturates when maturity ≈ 0; NoFood is binary. `phase_ticks` is written but never read. | `phase.rs:68,114-135,161` | Founder noise reproduced (69–83% non-default at t=5–10, conditions spread over 6–8 types). By t=1500 one condition holds **89–94%** of active cells — evolution cleans the table up, exactly as `spec.md` intends | Same as B7 |
| B14 | high | **FIXED** | Phases and strategies are invisible by construction. `WorldSnapshot` exports only `(x, y, GenomeHash)` per cell plus decay/pheromone/toxin maps: no `active_phase`, energy, strategy or lineage. The renderer draws only hash color. | `world.rs:58-60`, `render/mod.rs` | — | "phases aren't visible" |
| B8 | high | **FIXED** | Colors carry no lineage. Hue/sat/val come from bits of the FNV-1a hash of all 64 bytes. The code comment claiming lineage drift is visible is false. | `render/color.rs:16-18`, `genome.rs:115` | Before: one byte ±1 moved hue **87.6°** vs **90.0°** between unrelated genomes. After: **14.6°** vs **77.2°**, and **100%** of siblings stay inside their strategy's hue band | "mixed-color colonies" |
| B4 | medium | **FIXED** | Corpse deposit is `abs(energy)*0.5` at death, so starvation deaths leave ≈0. Combat deaths leave half the *overkill* (arbitrary, up to ~127). The larger problem is B1 wiping decay every tick. | `tick.rs:250` | Before: 5.9 per death, almost all combat overkill. After: every corpse leaves `corpse_biomass` (25) plus half its remaining energy | "no scavengers emerging" |
| B9 | medium | CONFIRMED-static | ~15 genes do nothing but still take top-N slots and cost metabolism. **Never read:** MAX_AGE (so no senescence or turnover), DORMANCY_TRIGGER, DORMANCY_COST, SENSE_PRIORITY, SWARM_SIGNAL, DECAY_RATE (gene), ADAPTATION_RATE, GENE_LINKAGE, HORIZONTAL_TRANSFER, TRANSPOSON_RATE. **Only in decode/spawner:** PREDATION_EFFICIENCY, ADHESION, OFFSPRING_SCATTER, TERRITORIAL_RADIUS, BASE_METABOLISM. `Cell.memory_dir` is never written. | grep over non-test code; `actions.rs:272` | — | Drives B5 and G1 |
| G1 | medium | CONFIRMED | Top-N gating runs over all 46 genes, so random parameter genes displace acquisition genes, which then decode ×0.1. The spawner's viability floor applies to **raw** bytes before antagonism and gating, so the "guaranteed" acquisition gene can still decode to ~0. | `genome.rs:252`, `spawner.rs` `random_genome` | Mean decoded photo at spawn 0.032; most founders classify as "none" (1359/2000, 2304–2747/4992) | Photo niche appears only after heavy selection |
| B13 | medium | CONFIRMED-static | Y is toroidal too, so the vent row (bottom) is adjacent to the full-sun row (top). The Beer-Lambert scan still starts at y=0. The vertical niche gradient has a seam. | `world.rs:168-170` | — | Possibly "flocks migrate to bottom-right and die" |
| B10 | low | CONFIRMED-static | `cell_ids()` is O(pool × free_list) because of `free_list.contains`, and `decode` runs ~5× per cell per tick. This will limit long runs once cells survive. | `world.rs:380-382` | — | Performance |

### Step 16 — four mechanism changes, judged by the invasion rates (2026-09-27)

Step 15 showed both consumers fail grow-from-rare (predator r = −0.0015, scavenger
r = −0.0029 per capita-tick). This step built four changes and judged each by
whether either sign flips. **None flips either one.** The predator's rate moves only
downward (capping its transfer makes it much worse); the standard scavenger
invasion is bit-for-bit unaffected by all four, and the reason is structural, not a
tuning miss (see #2 and #4). What each change did instead is below.

**Protocol.** Every change except #1 sits behind a `WorldConfig` knob whose default
is the old behaviour. Knob-off bit-identity was checked with the new `lab --hash`
(full state: every live cell's fields and record, every tile field) on 4 configs
(`WorldConfig::default()`, `archetypes.json`, `random_clusters`, the three step-14
switches on) x 2 seeds x 600 ticks = 56 checkpoints, against the post-#1 tree:
**identical after #2, after #3 and after #4**, and every checkpoint after t=0
diverges with the knob on (except where the knob cannot act, noted below). Each
change has a test that fails on the old behaviour (checked by disabling the new
path and running it). Measurements: the four step-15 commands, one arm per setting,
each arm paired with the baseline on the same seeds (seeding is untouched, so
founders are identical; t=0 hashes match). The sim is deterministic, so the saved
baseline arm is the `--vs-set` A arm — confirmed at the end by an actual `--vs-set`
run whose A arm reproduced the baseline seed for seed.

`n` for every score row is 10 seeds (± is the paired SE); invasions are 6 seeds.

**#1 — stillbirth bug (fixed; default changed).** `actions::offspring_energy_share`
bounds the gene to `[min_offspring_energy_share, max_offspring_energy_share]`
(new config, defaults 0.05 / 0.95); inside the bounds the gene is used as it is.
Bounding only the exact ends is not enough, measured with a probe over
`WorldConfig::default()`, seeds 1–3 x 1000 ticks, counting children that die in the
tick they are born:

| bound | `stillborn` + `childbirth` deaths | children dead in their birth tick |
|---|---|---|
| old `[0, 1]` | 23–62 + 82–3 566 per seed | 0.96–4.36% of births |
| `[0.01, 0.99]` | 0 | 0.70–2.38% (renamed starvation; births and starvation deaths up 30–70%) |
| **`[0.05, 0.95]`** | **0** | **0.02–0.09%** |
| `[0.1, 0.9]` | 0 | 0 |

0.05 is the narrowest bound that closes the leak: at `reproduction_energy_floor`
(40) a child starts with 2 energy, above a typical first-tick drain. Over all 10
seeds x 2000 ticks: old 940 stillborn, 72 412 childbirth deaths (6.7% of all
deaths), 1.15% of births dead at birth; new 0, 0, 0.03%. Births fall 32% (1.11M →
0.75M): on seed 1 half the population had shares in [0.95, 1) and bred itself to
death. Test: `neither_child_nor_parent_is_born_dead_whatever_the_share_gene`.
`a_scavenger_standing_on_decay_actually_eats_it` had to start below the
reproduction floor: its cell used to split during the test tick too, unnoticed while
a child could be born with 0.1 energy. The four commands re-run on this tree are the
new baseline (`base1` below). Effect: the default world loses 489 ± 310 scavengers
(the stillbirth pump was feeding them); nothing else moves beyond noise.

**#2 — trophic transfer and corpse energy (`max_predation_efficiency` 1.0,
`corpses_keep_energy` false).** A kill pays `predation_efficiency *
max_predation_efficiency` of the victim's pre-blow energy. With
`corpses_keep_energy`, what the killer does not take is laid on the victim's tile
as decay, and a cell dying of old age leaves `corpse_energy_fraction` of the energy
it held (`EnergyResult::senesced_energy`, deposited in the energy phase; liveness is
`energy > 0`, so the corpse cannot carry it to cleanup). All three deposits go
through the new `World::deposit_decay`. Tests:
`a_capped_kill_pays_the_killer_its_share_and_leaves_the_rest_in_the_body`,
`an_old_corpse_leaves_its_remaining_energy_too`. Not changed: a cell killed by
retaliation still leaves only biomass.

- **Capping the transfer destroys the archetype web and makes the predator worse,
  monotonically** (pred r −0.00127 → −0.00265 / −0.00268 / −0.00275 / −0.00320 at
  cap 0.5 (alone) / 0.5 / 0.25 / 0.1 (with kept energy); 5–6 of 6 invasions
  extinct). Mechanism, measured on archetype seed 1 with the cap alone: kills per
  tick 18 → 29 (t=100) and 32 → 58 (t=200), predators in their hunger slot
  146 → 215 and 68 → 122, world extinct at t=362. A capped kill fills a predator
  half as much, so the hunger slot (attack doubled below the energy trigger, step
  12) stays on and it keeps killing until the producers are gone. At 0.1 the
  predators cannot pay at all and die first, which is why that arm keeps three
  groups. The cap-only arm matches cap + kept energy, so the collapse is the cap's.
- **Kept corpse energy alone:** archetype scavengers +80 ± 13, effective groups
  +0.10 ± 0.02, producers −264 ± 112 (mechanism not measured; candidates: decay
  shading the 9-row photic band — `decay * 0.004` absorption — or scavengers taking
  the tiles); persistence and both invasion rates unchanged.
- **The standard scavenger invasion cannot see #2 at all** (bit-identical). The
  resident producers' `max_age` decodes to a 1 316-tick lifespan, so the first
  old-age corpses fall at t≈1316; the invaders arrive at t=1000 and are extinct by
  t=1125–1147. There are no kills in that world. `resident_before` is identical in
  both arms.

**#3 — Flee (`flee_can_escape` false).** Flee fires with probability `speed *
flee_response`, only from the nearest non-kin whose blow beats the cell's armour
or whose venom gets through its membrane (`nearest_danger`, judged with
`resolve_attack` against the cell's phase-modified armour; `sense_cached` fills
`SenseResult::threats` with each non-kin's phase-modified weapons, through the
tick's decode cache). And `resolve_all` resolves Flee *and* Move before Attack, so
a blow whose target has moved beyond the attacker's `attack_range` misses
(`TickStats::attacks_missed`, in the lab's `per_tick`). Moves go first for both
because resolution is simultaneous; a Move carries a cell away as much as a Flee.
Tests: `a_cell_flees_only_from_what_can_hurt_it_and_only_as_often_as_it_can_move`,
`a_cell_that_flees_out_of_reach_is_missed`.

- **No effect on either invasion rate** (pred −0.00004 ± 0.00017; with
  `attack_only_when_harmful` too, +0.00006 ± 0.00021). The archetype prey are
  sessile, and in the default world flight hardly exists: the old gate made Flee
  0.03–0.11% of actions (sessile mutants with a flee byte "fleeing" with no speed),
  the new one rounds to 0.000%, and 0.004–0.08 attacks miss per tick (seed 1). The
  predator is not failing because prey escape.
- Archetype runs are the regression check: with the flag on they diverge only
  because those phantom flights stop (0.028% of actions → 0; no attack missed).
  The +0.20 ± 0.13 groups persisting there is not a flee effect.
- `attack_only_when_harmful` (step 14, measured here as #3's partner, since without
  it a prey adjacent to a predator takes the Attack gate before Flee): archetype
  hunters +36 ± 21, scavengers +69 ± 13, effective groups +0.11 ± 0.03; default
  world: hunters persist on 1 seed of 10 (0 before). Mechanism not measured. It
  still ignores venom (a weak venomous attacker counts as harmless).

**#4 — foragers stay on food (`foragers_stay_on_food` false).** `sense` computes
`own_food_share` = own tile's food / (own + richest other food tile in range), on
the food scale the scan already uses (decay, light, vent zone; 0 for a hunter),
and the Move chance is scaled by `1 − chemotaxis_strength × own_food_share`.
Test: `a_forager_stays_on_a_meal_and_leaves_an_empty_tile`.

- **Default world: scavengers +404 ± 190, effective groups +0.15 ± 0.07.** On
  seed 1, scavenging income per cell-tick is 0.13 / 0.30 / 0.43 at t=500 / 1000 /
  1500 against 0.10 / 0.13 / 0.12 without it; scavengers 1 095 vs 350 at t=1000.
- **Archetypes and both standard invasions: unchanged** (the archetype scavenger
  has speed 0 and the predator's food is never underfoot). The flag cannot act on
  a sessile cell.
- **Extra run: a mobile scavenger** (the archetype with speed 100, chemotaxis 200,
  sense 128 bytes — decoded speed 0.26, chemotaxis 0.78; `--invade <hex> --rows
  0,16`, same residents; the archetype hex through this path reproduces `--invade
  scav` exactly). **It grows from rare — r = +0.440 ± 0.004, past the rare cap on
  6/6 — and then crashes**: it eats the standing decay (≈13 000 scavenged per seed
  vs ≈500 for the sessile invader), peaks at ~245 cells and is extinct by t≈1300–1450
  on 6/6. So mobility flips the sign of r but the invader does not persist; r while
  rare alone would read this as a success. Extinct at t=2000: 6/6 with neither
  switch, 6/6 with `foragers_stay_on_food`, 5/6 with `corpses_keep_energy`, **3/6
  with both** (survivors 20–24 cells). Six seeds; suggestive only.

**Summary** (each arm against the post-#1 baseline, except row 1; ± paired SE;
invasion cells: mean r, paired difference, seeds extinct):

| arm (vs base) | default: groups persisting | default: hunter persists | default: scav mean | arch: groups persisting | arch: hunter mean | arch: scav mean | arch: photo mean | pred invasion r (paired diff), extinct | scav invasion r (paired diff), extinct |
|---|---|---|---|---|---|---|---|---|---|
| #1 share bounds (vs pre-fix) | -0.20 ± 0.25 | 0/10 | -489 ± 310 | +0.10 ± 0.10 | +8 ± 9 | -12 ± 7 | -86 ± 101 | -0.00127 (+0.00021 ± 0.00022), ext 0/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #2 corpses_keep_energy | +0.10 ± 0.18 | 0/10 | +94 ± 84 | +0.10 ± 0.18 | +8 ± 13 | +80 ± 13 | -264 ± 112 | -0.00148 (-0.00020 ± 0.00025), ext 1/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #2 cap 0.5 only | +0.20 ± 0.20 | 0/10 | +286 ± 176 | -2.40 ± 0.31 | -39 ± 8 | -172 ± 16 | -3572 ± 108 | -0.00265 (-0.00138 ± 0.00012), ext 5/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #2 keep + cap 0.5 | +0.10 ± 0.23 | 0/10 | +386 ± 252 | -2.30 ± 0.21 | -39 ± 8 | -152 ± 25 | -3511 ± 131 | -0.00268 (-0.00141 ± 0.00013), ext 6/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #2 keep + cap 0.25 | +0.00 ± 0.21 | 0/10 | +127 ± 153 | -3.10 ± 0.28 | -39 ± 8 | -180 ± 24 | -3654 ± 156 | -0.00275 (-0.00148 ± 0.00012), ext 6/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #2 keep + cap 0.1 | +0.10 ± 0.18 | 0/10 | +360 ± 146 | -0.80 ± 0.13 | -39 ± 8 | +327 ± 29 | -1015 ± 162 | -0.00320 (-0.00193 ± 0.00014), ext 6/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #3 flee_can_escape | -0.10 ± 0.18 | 0/10 | +444 ± 433 | +0.20 ± 0.13 | +8 ± 7 | +19 ± 8 | -32 ± 96 | -0.00131 (-0.00004 ± 0.00017), ext 1/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| attack_only_when_harmful | +0.00 ± 0.00 | 1/10 | +137 ± 91 | +0.10 ± 0.10 | +36 ± 21 | +69 ± 13 | -157 ± 179 | -0.00136 (-0.00008 ± 0.00020), ext 0/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #3 flee + harmful | -0.20 ± 0.20 | 0/10 | +106 ± 106 | +0.00 ± 0.15 | +46 ± 22 | +86 ± 21 | -246 ± 189 | -0.00121 (+0.00006 ± 0.00021), ext 0/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| #4 foragers_stay_on_food | +0.10 ± 0.23 | 0/10 | +404 ± 190 | +0.00 ± 0.00 | +0 ± 0 | +0 ± 0 | -1 ± 1 | -0.00127 (+0.00000 ± 0.00000), ext 0/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |
| keep + stay | +0.00 ± 0.21 | 0/10 | +312 ± 158 | +0.10 ± 0.18 | +5 ± 13 | +77 ± 13 | -275 ± 113 | -0.00148 (-0.00020 ± 0.00025), ext 1/6 | -0.00293 (+0.00000 ± 0.00000), ext 6/6 |

**What the predator's failure is not, and what it may be.** Neither faster transfer
(capping it hurts) nor escaping prey (nothing escapes) explains it. 70% of dead
predator invaders die of **old age** (259 of 372), births 0.0019 vs deaths 0.0032
per capita-tick. The archetype predator's `max_age` is top-N gated to 0.038: it
lives 401 ticks, matures at 200 (the lifespan cap) and waits 100 between births, so
it has at most two breeding windows, and it averages about 0.8 births per life. That
is the reproduction brake step 12 installed, and step 15 found selection shortening
maturity. Not yet measured: whether the brake, rather than energy, is what binds.

**Default proposals (not flipped):**
- `corpses_keep_energy` → **true**: it is what `spec.md` already says (old corpses
  leave `corpse_energy_fraction` of their energy); archetype scavengers +80 ± 13 and
  effective groups +0.10 ± 0.02 with no loss of persistence; its cost is producers
  −264 ± 112 in the archetype world.
- `foragers_stay_on_food` → **true**: default-world scavengers +404 ± 190 and
  effective groups +0.15 ± 0.07, with the income channel it targets rising 2.5–3.6x
  on seed 1; no effect where it cannot act.
- Together (`--vs-set` both): default scavengers +312 ± 158, archetype scavengers
  +77 ± 13, producers −275 ± 113, persistence and invasion rates unchanged.
- `max_predation_efficiency` stays 1.0; `flee_can_escape` has no measured effect
  (it only removes sessile "flights"), so no measurement backs flipping it.

### Step 15 — a measurement base, and what it shows (2026-09-27)

An approach review (three independent lenses plus a critic, all checked against the
code) concluded that the limiting factor was the measurements: 1–3 seeds, snapshots,
gene-cutoff classes, and a harness that lived out of tree and had drifted from the
sim. This step fixes that before any more mechanism work. **No simulation behaviour
changed** — verified with full-state hashes (every cell, every field map) at 12
checkpoints on 3 configs x 2 seeds, before and after, identical.

**Built:**

| Change | Where | Test |
|---|---|---|
| Config files load again. `WorldConfig` had no `serde(default)`, so step 14's four fields made `archetypes.json` and `default.json` fail to parse ("missing field `max_move_distance`"). Now `#[serde(default, deny_unknown_fields)]`: missing fields default, misspelled ones are an error. | `config.rs` | `a_config_file_missing_newer_fields_still_loads` (fails on the old code), `a_misspelled_config_field_is_an_error` |
| The sim counts its own events: `World.stats: TickStats` (actions, births, deaths by cause, attacks, energy per channel incl. predation, metabolism, cap waste, dormancy, phase transitions, per-phase wall time), reset every tick. `World.records` (beside the pool) keeps each cell's founder lineage, lifetime income per channel, upkeep and cause of death. Nothing in the sim reads either. | `sim/stats.rs`, `world.rs`, `energy.rs`, `tick.rs`, `actions.rs`, `spawner.rs` | `every_birth_and_death_is_counted_and_every_death_has_a_cause` (population(t+1) = population(t) + births − deaths every tick, no unexplained death, no living cell carrying a death label), `a_death_is_attributed_to_the_drain_that_caused_it`, `diet_is_the_channel_that_paid_most_once_it_paid_enough` |
| Every founder has a lineage under every seeding strategy (uniform: one each; clusters: one per cluster; archetypes: 1–4), inherited on every birth. | `spawner.rs`, `actions.rs` | `lineage_is_inherited_and_injection_starts_a_new_one` |
| `Simulation::inject` / `spawner::inject`: introduce a batch as a new lineage mid-run, placed by the sim's own RNG. | `mod.rs`, `spawner.rs` | same |
| Library crate + `src/bin/lab` (see section 2), `default-run = "primordium"` so `cargo run` still opens the window. | `lib.rs`, `bin/lab/*`, `Cargo.toml` | 8 lab unit tests |

The old harness's four known defects are gone because the numbers now come from the
sim: maturity (`gene*1000` vs the sim's capped mapping), vent income (ignored the
crowd), "starvation" (included old age), lineage (never set under uniform seeding).
An independent review of this step found 20 real issues, all fixed — among them:
kills were counted when a blow landed although a struck cell can be revived later in
the same tick (by kin sharing, or by absorbing its own kill), so deaths are now counted
at cleanup from the cause label and the energy phase clears stale labels; the
invasion rate telescoped to an endpoint and is now measured only while rare.

**Two death paths nobody had counted.** Deaths with no known cause (~1%) turned out to
be births: `child_energy = parent.energy * offspring_energy_share`, so a share that
decodes to 0 makes a zero-energy child — counted as a birth, dead at cleanup, leaving
`corpse_biomass` (25) of decay: **energy from nothing (bug, not fixed)**. A share of 1.0
kills the parent (0.35 deaths/tick, ~2% of all deaths, on uniform seed 42 at t≤400).

**Baseline, your defaults** (`WorldConfig::default()`, 10 seeds x 2000 ticks, window
t=1000–2000, n_min 10; class = what cells live on):

| seed | pop | photo | thermo | scav | hunter | groups persisting | eff. groups | rows_90 |
|---|---|---|---|---|---|---|---|---|
| 1 | 5769 | 2588 | 42 | 2661 | 12 | 3 | 2.11 | 35 |
| 2 | 3911 | 2850 | 0 | 1018 | 6 | 2 | 1.80 | 10 |
| 3 | 3408 | 3095 | 37 | 246 | 2 | 3 | 1.38 | 14 |
| 4 | 4168 | 3615 | 31 | 238 | 3 | 3 | 1.33 | 32 |
| 5 | 3877 | 3503 | 1 | 517 | 0 | 2 | 1.47 | 14 |
| 6 | 3301 | 3016 | 10 | 526 | 0 | 2 | 1.55 | 14 |
| 7 | 7588 | 3817 | 4 | 3040 | 0 | 2 | 1.98 | 33 |
| 8 | 4071 | 3728 | 2 | 392 | 9 | 2 | 1.39 | 16 |
| 9 | 4465 | 3244 | 65 | 1468 | 0 | 3 | 1.98 | 15 |
| 10 | 4393 | 3677 | 37 | 535 | 1 | 3 | 1.53 | 12 |

(group columns are window means). Persisting: photo 10/10, scav 10/10, thermo 5/10,
**hunter 0/10**. Groups persisting 2.50 ± 0.53. At t=200 on seed 42, 62 cells had
predator-dominant genes but only 2 had lived on predation.

**`archetypes.json`, same protocol:** all four persist on 7/10 (hunter 8/10, thermo
9/10); photo ~3 900, thermo ~220, scav ~240, hunter 10–71; `rows_90` 8.6–8.9.

**Frozen-evolution control** (`archetypes.json`, arm B = `max_mutation_rate`,
`max_transposon_rate`, `max_horizontal_transfer` all 0; 10 paired seeds): groups
persisting 3.70 → **2.20** (paired diff −1.5 ± 0.22), hunter persisting 8/10 → **1/10**
(mean −16 ± 6 cells), thermo 9/10 → 1/10. The thermo loss is a **cohort die-off**: the
vent founders are clones with one lifespan, the vent zone is full so the cohort never
turns over (474 cells, flat, t=900–1300), and it dies of old age together at t≈1350
(474 → 8). Mutation staggers lifespans and hides this.

Per-lineage traits (`--traits`, every 250 ticks):

| run | predators t=0 → 1000 → 2000 | maturity (ticks) | hunger threshold | sated attack | prey armour |
|---|---|---|---|---|---|
| frozen, seed 1 | 1250 → 31 → **5** | 200 fixed | 188 fixed | 67.6 | 80.0 |
| normal, seed 1 | 1250 → 45 → 21 | 200 | 188 → 186 | 66 → 69 | 80 → 79.3 |
| normal, seed 3 | 1250 → 43 → **85** (98 at 1750) | 200 → 168 → **101–124** | 188 | 66 → 69 | 80 → 79.4 |

**The designed predator is a slow drain, not a population.** Without evolution it
dies out; where it recovers, the trait that moved is maturity. Step 12's "predators
creep up over thousands of ticks, consistent with the hunger trigger drifting toward
greed" is corrected: in these runs the hunger byte does not move; maturity halves.

**Grow from rare** (introduced at t=1000, 30 cells, into the archetype's own band,
measured while rare, to t=2000):

| invader | residents (shares) | r per capita-tick | grew | extinct | how they died |
|---|---|---|---|---|---|
| predator | photo/vent/scav 0.6/0.2/0.2 | **−0.00149 ± 0.00013** | 0/6 | 1/6 (others 4–7 cells left) | — |
| scavenger | photo/vent 0.75/0.25 | **−0.00293 ± 0.00003** | 0/6 | 6/6, ~130 ticks after arrival | 122/122 starved; 85% of their income was light |

Both consumers fail the standard coexistence test under current mechanics. The
scavenger's failure has a mechanism visible in the code: a sessile cell scavenges only
its own tile (`scavenge_income` reads the cell's tile), decay only fades where it lies
(`diffusion::fade_decay`, no transport), and corpses land where the dead cell stood —
so after the t=0 detritus windfall a sessile scavenger eats only what dies under its
own children. **Not yet measured:** whether letting it move toward food, or letting
decay spread, changes the sign.

### Step 14 — `adaptation_rate`, and three movement switches (2026-09-27)

**`adaptation_rate` (gene 38) is implemented — every gene is now read.** `spec.md`:
"speed of within-lifetime epigenetic-like modifier shifts. Not inherited." Built as
**thermal acclimation**: each tick a cell closes `adaptation_rate * max_adaptation_rate`
(config, default 0.02) of the gap between its effective temperature preference and
its tile's temperature, and pays `temperature_mismatch_cost` against that acclimated
preference. The shift is `Cell::temp_acclimation`, starts at 0 in every newborn, and is
never written to the genome. Code: `energy::acclimate`,
`energy::acclimated_preference`, applied in `tick::phase_energy_update`. Test:
`a_cell_acclimates_to_its_water_at_its_own_rate_and_passes_none_of_it_on`.
Not yet measured at the population level.

**Three movement switches, built and tested, all OFF by default (behaviour
unchanged), not yet measured:**

| Config field | What it does | Code | Test |
|---|---|---|---|
| `max_move_distance` (1) | A move goes `max(1, round(speed * max))` tiles along its heading, stopping at the first occupied tile and never past the sense radius. `speed` stays the *chance* of moving (spec gene 6). | `actions::extend_move`, `SenseResult::free_run` | `a_fast_cell_covers_several_tiles_but_never_through_another_cell` |
| `attack_only_when_harmful` (false) | Attack only a threat whose armour the blow can beat; otherwise fall through to Flee. Today a harmless cell with a predator adjacent "attacks" it instead of running, because Attack is gated before Flee. | `decide` gate 2, `SenseResult::nearest_threat_armor` | `harmless_prey_runs_from_an_adjacent_threat_instead_of_fighting_it` |
| `food_targets_richest` (false) | Steer toward the richest food in sense range (most decay / light), nearest on ties, instead of the nearest trace. | `sense` food loop | `a_forager_can_steer_to_the_richest_food_rather_than_the_nearest_trace` |

Documented in `spec.md` (World Parameters) and `architecture.md`. To evaluate: compare
each switch, and all three together, against the defaults on `default.json` and
`archetypes.json`, watching `per_class` (especially scavenger/hunter numbers and
speed) — short runs, few processes.

**Fresh 10k baseline on the user's defaults** (`default.json` = `WorldConfig::default()`,
uniform seeding) was run and finished, but its results lived in the wiped scratch dir
and are not recorded; the interim readings were seed 2 at t=10 000: photo 252 /
scav 2 350 / hunter 541, seed 3: photo 3 390 only, seed 1: 24 586 cells at t=5 000
(20 289 scavengers). Re-run it before quoting a baseline.

### Step 13 — `random_clusters` collapsing (2026-09-27)

Re-measured before touching anything: on the old baseline file (clusters, 5 000
cells, maturity cap 1 000) 4 of 5 seeds collapse to 8–35 cells by t=1 500.

- **Not the temperature fix.** With `temperature_mismatch_cost` 0 it still collapses
  on 3 of 4 seeds.
- **It is the seeding.** The same world seeded uniformly holds 3 341–3 900 cells with
  3–4 classes on all three seeds.
- **Founder determinism, the problem from the very first brief.** Seed 3's founders
  contain **2 photosynthesizers out of 4 992**; seed 2's sit in clusters deep in the
  dim rows (net −0.33 at spawn). Sixteen clusters are sixteen ancestor genomes in
  four fixed rows, and since the G1 fix each ancestor is a pure specialist, so whether
  any cluster is a *lit* producer is down to the roll. `spec.md` accepts that clusters
  in the wrong place die ("mismatched placement creates immediate directional
  selection pressure") — so this is the design working, not a defect.

**On your current defaults it does not happen.** `random_clusters` with 1 000 cells
and the 30-tick maturity cap survives **5 of 5 seeds** to t=3 000 with 2–3 classes
each (e.g. photo 448 / scav 772 / thermo 26; hunter 992 / scav 212 / thermo 16).
Raising `cluster_count` to 64 gives bigger but photo-dominated worlds (2 089–4 322),
so 16 stays. **No code change.**

`default.json` had drifted from its documented meaning ("equals
`WorldConfig::default()`") and is now regenerated from the code; the old one is kept
as `step6-baseline.json` so the step 3–10 numbers stay reproducible.

### Step 12 — a seeded four-way food web that holds (2026-09-27)

Target: your defaults (`random_uniform`, 1 000 cells, `max_maturity_ticks: 30`),
with `preset_archetypes` on top. **Result: all four seeded lineages coexist to
t = 4 000 on 6 of 6 runs (equal and pyramid shares, 3 seeds each), and to t = 8 000
on 2 of 3.** Run it with `cargo run --release -- docs/handover/archetypes.json`.

| run | t = 4 000 (photo / vent / scav / pred) | t = 8 000 |
|---|---|---|
| pyramid seed 1 | 4160 / 184 / 407 / 10 | **4284 / 173 / 283 / 10** |
| pyramid seed 2 | 2154 / 93 / 229 / 42 | collapsed (predator outbreak ~t=5 000) |
| pyramid seed 3 | 4190 / 269 / 390 / 12 | **3670 / 180 / 147 / 118** |
| equal seed 1 | 3235 / 242 / 1243 / 44 | — |
| equal seed 2 | 3643 / 179 / 897 / 33 | — |
| equal seed 3 | 1957 / 83 / 496 / 137 | — |

The failure mode is evolutionary: predator numbers creep up over thousands of ticks,
consistent with selection pushing the hunger trigger toward greed, until they
overshoot. Predators show as class `none` in the report because their base attack
(0.27) is below the classifier's 0.35 cutoff; `xtab` by lineage is the right view.

**How it got there — each step measured, each wrong turn recorded:**

1. **Producers alone were already fine** under the new decode order (photo ~4 875 +
   vent ~473, stable). The breakage was all predators.
2. **Predators went 250 → 1 587 in 10 ticks.** Damage, armour and cooldown sweeps all
   ended extinct by t=375. The trace showed why: a newborn has no cooldown, and with
   maturity capped at 30 it breeds the moment it has fed, so every newborn goes
   kill → breed → kill and breeding *chains* double every ~5 ticks. Cooldown limits
   an individual, not a chain. **Maturity is the only brake on that**, and the 30-tick
   cap removes it: at 30 predators always exploded, at 100–250 they starved out after
   the opening, at 300 seed 1 oscillated (hunters 249 → 4 → 65 over 1 400 ticks).
3. **Your 30-tick cap is right for random seeding** — measured as good or better than
   300 (four classes at t=2 000 on 2 of 3 seeds at both). So the default is unchanged
   and the archetypes carry their own config (`docs/handover/archetypes.json`,
   `max_maturity_ticks: 300`).
4. **Surplus killing.** Full predators kill anyway (115/tick in the opening) and the
   energy is wasted at their cap. The genome can express hunger with no new rule: a
   base attack that does nothing through prey armour, plus one phase slot
   (EnergyLow → offense ×2). That cut opening kills to 13–29/tick. It first failed
   because sated predators still did 7 damage through armour 60 — sated damage must
   be exactly zero (armour 80 vs a sated bite of 67).
5. **Synchronised maturation.** All 250 founder predators reach maturity on the same
   tick and, with no cooldown, breed repeatedly: 249 → 1 130 in 25 ticks. A 100-tick
   cooldown stops it.
6. **Hunger threshold.** Firing below 60% of cap still out-grazed the prey (5 661 →
   1 836 over 650 ticks); below ~20% predators starved out on every seed. 25%
   (`PREDATOR_HUNGER_THRESHOLD` 188) is the value shipped; 180–192 all behave alike.
7. **Scavengers were being eaten, not starving** — 750 → 180 in 25 ticks at 53
   kills/tick. A mobile cell cannot carry protective armour (`armor ↔ speed`).
8. **Then they could not reproduce.** With the predator told to leave them alone they
   survived the opening but sat at break-even with **0–2% ready to reproduce**, aging
   out while uneaten decay piled up around them (5 → 10.7 per occupied tile).
9. **The scavenger that works is the one selection builds.** Under random seeding the
   thriving scavengers are nearly sessile (speed 0.01–0.02) and earn ~2.2/tick from
   ~1.6 decay — i.e. they also photosynthesize. The archetype is now a sessile
   mixotroph: scavenging strongest, photosynthesis as a second income, armoured
   (free when sessile), low reproduction threshold.
10. **And it has to share the light.** In its own band beneath the photosynthesizers
    it earned 0.17 against an upkeep of 0.88 — shaded out. Interleaved in the photic
    rows it holds 229–1 243 at t=4 000.

**A measurement discarded.** One long run reported zero scavengers everywhere. The
lab sync had failed on a stale anchor, but its output was piped through `tail`, the
failure was masked, and the runs used the old binary. `sync-lab.sh` status is now
checked explicitly.

**Tests:** `the_predator_is_hungry_only_when_it_is_actually_low`,
`every_archetype_clears_its_own_upkeep_in_its_own_niche` (now: zero sated damage, a
real bite when hungry), `a_predator_reads_the_other_archetypes_as_prey_but_not_its_own_kin`
(including 20 generations of drift), and the band-layout test.

**Your two questions from the window, answered with numbers** — "are scavengers
attracted to food, and afraid of predators?" and "wouldn't more of both fix them?":
the first archetype scavenger *was* built for maximal attraction and it is the one
that failed (upkeep 1.33–1.37 vs 0.88 sessile; attraction acts only on ticks the Move
gate fires, one tile at a time, toward the *nearest* decay trace rather than the
richest). More fear of non-kin would mean fleeing its own food, and fear barely works
anyway because **Attack is gated before Flee**, so a harmless cell with a predator
adjacent "attacks" instead of running. Those two mechanics — attack-before-flee and
nearest-not-richest — are the real levers, and they are tested with the movement
decision below.

### Step 11 — the tick now costs what the population costs (2026-09-27)

**Found on the way: the temperature map was being erased (FIXED).** `spec.md` calls
temperature "static (Perlin noise at init), rarely changes". Measured, 64x64:
mean **219.8 at t=0, 170.2 at t=50, 71.3 at t=150, 0.0 at t=300**. Temperature is a
`u8` and the diffusion write-back used `as u8`, which floors, so a tile whose update
came to 127.99 became 127 — a full degree per tick, about 10 000x the configured
`temperature_decay`. **Every run in this project past tick ~300 has had no
temperature map**, and every cell has been paying its whole `temperature_preference`
as mismatch cost. Now `.round()`. Test:
`the_temperature_map_stays_put_under_its_configured_rates`.

**Profile first.** `perf` is blocked in this sandbox, so the lab now times each tick
phase directly (`phase_pct`, `ms_per_tick` in every report line). At pop 274–369
on `default.json`:

| phase | share |
|---|---|
| diffusion | 38–52% |
| sunlight | 22–31% |
| sense + decide | 4–19% |
| energy | 6–13% |
| prepare_next | 4–6% |
| cleanup | 3–4% |

Eighty-to-ninety percent of every tick was spent on work sized by the grid.

**Bit-identical changes** (verified: 6 runs, 2 configs x 3 seeds x 300 ticks,
every report field equal):

- `recompute_sunlight` walks row-major with one running intensity per column. The
  old column-major walk strode 512 tiles per step and missed cache on nearly every
  access. It also reuses one precomputed `exp(-water_alpha)` for plain-water tiles
  — `tile_absorption` adds exactly `0.0` for them, so the value is identical — rather
  than calling `exp` on all 262 144.
- `World::placed_in_next` lists cells on the next grid by walking the cell pool,
  sorted by tile index. Energy and cleanup used to scan every tile. **The sort is
  load-bearing**: cleanup frees dead slots in this order and the free list decides
  which ids later births reuse, which drives the RNG order of the whole run.
- Temperature diffusion is skipped when `255 * (decay + spread) < 0.5`: a tile then
  moves by less than half a degree and rounds straight back, so the pass is a
  provable no-op. At the defaults the bound is 0.28.
- The three `diffusion_a.clone()` per tick (1 MB allocate-and-copy each) are gone.
- The energy phase and corpse deposit use the per-tick decode cache.

Result: `default.json` **25.2 → 14.8 ms/tick**, your uniform config **17.9 → 9.5**.

**Changes that move results by a negligible amount** (deterministic; same seed still
gives the same run):

- `DECAY_FLUSH` (0.01): decay below it is cleared. Multiplicative fade never reaches
  zero, so with `initial_decay_matter` every tile carried a trace forever and none
  could take the plain-water path. 0.01 of decay scavenges to 0.009 energy against
  an upkeep near 1.
- `FIELD_FLUSH` (1e-4) for pheromone and toxin, four orders of magnitude below what
  a cell could sense.
- Rows whose three-row neighbourhood is all zero are written as zero and skipped;
  every row that *is* computed is computed exactly as before (test
  `skipping_empty_rows_changes_nothing_that_was_computed`).
- A field that is zero everywhere skips gather, diffusion and scatter entirely —
  toxin's normal state, since it only forms where ≥10 cells died close together.

Result: **`default.json` 25.2 → 5.7 ms/tick, about 4.4x.** On the uniform config at
4 000 cells, diffusion is down to 18% and sense/decide (37%) and energy (18%) lead:
the cost now scales with the population instead of with the grid.

**Harness:** rebuilt from scratch after `/tmp` was wiped, via a new
`sync-lab.sh` that **fails loudly on a rejected hunk** (a silent reject once cost a
wrong measurement). `lab --dump-config` prints `WorldConfig::default()` so config
files can be regenerated from the code; `docs/handover/default.json` was refreshed
this way (ten fields had been added since it was written). Patch regenerated and
verified.

**Note on your defaults.** `config.rs` was edited on 2026-09-22 (after the last
session) to `random_uniform`, 1 000 cells and `max_maturity_ticks: 30`. Those are kept.
`default.json` keeps its older choices (`random_clusters`, 5 000 cells, maturity
1 000) so that earlier measurements stay comparable; `play.json` in the lab is the
code defaults, and is what the balance work below targets.

### Step 10 — gating before antagonism, and the first multi-class worlds (2026-09-21)

**Adopted, with the user's approval, as a deliberate design change.** The decode
pipeline is now:

```
normalize -> top-N gating -> antagonistic pairs -> physical caps
```

It was `normalize -> antagonistic pairs -> top-N gating -> physical caps`, which is
what `architecture.md`, `CLAUDE.md` and the old `genome.rs` comment all said. All
three are updated, and `spec.md` now states the order explicitly under Expression
Constraints, because it is load-bearing.

**Why.** `spec.md` describes three *independent* anti-supercell mechanisms
(metabolic budget, top-N gating, antagonistic pairs). Running the pairs first
coupled two of them: a gene was cut by its partners, then cut **again** by the
falloff, because those cuts had cost it its rank. `speed` is in three pairs —
`photosynthesis_rate`, `armor`, `adhesion`, more than any other gene — so it took
the worst of it:

| | mean `speed`, 20 000 random genomes |
|---|---|
| raw byte | 0.498 |
| after antagonistic pairs | 0.138 |
| after top-N gating | **0.015** |
| | **99.8% of genomes gated** |

A cell at 0.015 moves once every ~66 ticks. Both mobile niches — scavenging and
predation — depend on movement, so **both were closed to every genome the world
could roll**, which is why only photosynthesizers and vent-feeders ever survived.

Gating first also means a gene the cell does not express exerts no antagonistic
pressure, which is the physically coherent reading: armour a cell is not growing
should not be slowing it down.

**Measured, `default.json --uniform`, 3 seeds, t=10 000:**

| seed | photo | thermo | scav | hunter | classes |
|---|---|---|---|---|---|
| 1 | 3517 | 11 | — | **42** | 3 |
| 2 | 2780 | 1 | **766** | **12** | **4** |
| 3 | 3439 | — | — | — | 1 |

Hunters carry positive net on both seeds that have them (+0.008, +13.460) and
persist to 10 000 ticks. Before the change the same runs gave photosynthesizers
only, or photosynthesizers plus a handful of scavengers on their way out.

**Section 6's target — three or more strategy classes coexisting — is met on 2 of 3
seeds at 10k under uniform seeding.** It had been met on 1 of 5 at step 6, and that
one was drifting.

Test: `a_genes_own_investment_decides_whether_it_expresses_not_its_partners`, which
on the old ordering decodes `speed` to 0.0204 against a falloff of 0.1 — i.e. the
genome's single largest investment was gated out by its own partners.

**What this does not do.** It does not make `speed` large: the surviving
photosynthesizers still decode to 0.004-0.007, correctly, because they are sessile
by selection. It raises the *ceiling* for genomes that invest in movement, which is
what the mobile niches needed. The one-tile-per-tick cap from `spec.md` gene 6 is
untouched and is still the hard limit on how far anything can range.

**`random_clusters`, 3 seeds, t=10 000** (recorded at step 6: 207 / 1377 / 61):

| seed | pop | classes |
|---|---|---|
| 1 | **2688** | photo 1849 + **hunter 839** |
| 2 | 22 | thermo only — *worse* than the recorded 1377 |
| 3 | **3384** | photo 1357 + thermo 2 + **scav 1992** + **hunter 32** — four classes |

Two seeds improve by 13x and 55x and carry mobile classes for the first time;
decoded `speed` among them is 0.113-0.315 against 0.015 before, which is the change
doing exactly what it was meant to. Seed 2 collapses. **Do not read the step-6 table
as a baseline any more** — between G1, the dormancy caps and this, founder
composition and the whole economy have moved.

Seed 3's per-class `net` is negative across the board at t=10 000, so that world may
be in decline rather than at equilibrium. One snapshot cannot tell the difference;
it needs a longer run or a births-vs-deaths series.

**The archetype bands did not survive the change** (t=2000): equal shares goes
extinct outright, and the pyramid ends as 1063 cells that are all
`PRED->photo` — the predator lineage turned photosynthesizer again. The
hand-designed genomes were tuned against the *old* decode order, where their
near-zero filler genes kept everything they cared about inside the top N. They need
re-tuning, and `--archetypes` and `--niche` both need re-running for the same
reason.

### Step 9 — why only sessile strategies survive (2026-09-21)

From the user, running random seeds in the window: "the only cells appearing to
survive are photosynthesizers and some thermosynthesizers. scavengers and predators
die. and from the looks of it, scavengers don't appear to be eating at all."

**They are eating. They cannot reach the second meal.** New `per_class` block in the
lab report (income, upkeep, net, the decay on the cell's own tile, energy fraction,
decoded speed, and what share is reproduction-ready) — the grid-wide means hid this
completely, because they are dominated by whichever class is most numerous.

`default.json --uniform`, seed 1:

| t | scav n | scav `tile_decay` | scav net | photo `tile_decay` |
|---|---|---|---|---|
| 0 | 1765 | 8.00 | **+0.693** (the only positive class) | 8.00 |
| 50 | 519 | **0.38** | −1.815 | 2.87 |
| 100 | 33 | **0.01** | −1.423 | 2.56 |
| 300 | 8 | **0.00** | +0.168 | 1.47 |

At tick 0 the scavengers are the only class with a positive balance. Fifty ticks
later they are the only class standing on bare ground: they stripped their own tiles
and every surviving scavenger from t=100 on sits on **0.00** decay while the `none`
cells sit on 9.63 of it. A unit test (`a_scavenger_standing_on_decay_actually_eats_it`)
confirms the energy path end to end, so the mechanism is fine.

#### The cause: `speed` is the most suppressed gene in the genome

Decomposed over 20 000 random genomes:

| | mean `speed` |
|---|---|
| raw byte | 0.498 |
| after antagonistic pairs | **0.138** |
| after top-N gating | **0.015** |
| | **99.8% of genomes have `speed` gated** |

Two multiplications, 33x total. `speed` appears in **three** pairs — with
`photosynthesis_rate`, `armor` and `adhesion` — more than any other gene, and each
one is applied. That is documented as intended. But it drops `speed` to 0.138, which
then essentially never ranks inside the top 12 of 46, so it takes the 0.1 falloff as
well. A cell with a decoded speed of 0.015 moves **once every ~66 ticks**; it strips
its tile in three and starves for the other sixty-three.

Both mobile niches — scavenging and predation — depend on movement, so both are
structurally unreachable for a random genome. The *hand-designed* archetypes are
fine (scavenger 0.56, predator 0.63) precisely because their other genes are kept
near zero. **This is a random-seeding problem, not a scavenger problem.**

#### The existing knobs do not recover it

`top_n_falloff` and `top_n_gene_count` are config, so they can be tuned without
touching documented design. Sweeping them does not help (uniform seeding, t=800):

| falloff | count | pop | surviving scav | scav `speed` |
|---|---|---|---|---|
| 0.1 | 12 (default) | 3496 | 70 | 0.002 |
| 0.1 | 16 | 2020 | 10 | 0.002 |
| 0.3 | 12 | 2455 | 18 | 0.009 |
| 0.3 | 16 | 1398 | 76 | 0.011 |

Loosening the gate raises everyone's expression, so photosynthesizers gain too and
still win. Selection keeps choosing sessile genomes because photosynthesis pays
everywhere and costs no movement.

#### Three ways out, all design changes — for the user to choose

1. **Rank top-N *before* antagonism.** Expression capacity would then be about what
   the genome invests in, not about what survives its trade-offs. `spec.md` does not
   fix the order; `architecture.md:72` and CLAUDE.md both document it as
   antagonism-then-gating, so this is a documented-design change. Estimated effect:
   `speed` would be gated ~74% of the time instead of 99.8%.
2. **Take `speed` out of one of its three pairs.** `adhesion ↔ speed` is the weakest
   of them — `adhesion`'s distinct behaviour (sticking to kin) is now implemented
   separately in the Move gate, so the pair double-counts.
3. **Make `speed` mean distance rather than probability.** `spec.md` gene 6 defines
   it as "probability of moving each tick", which caps *every* cell at one tile per
   tick however the genes fall. This is the one that would change what the window
   looks like most.

None is made here.

#### Also fixed: antagonistic pairs were order-dependent

`spec.md` states the rule as `effective_a = raw_a * (1 - raw_b_norm * factor)` —
**raw_b**, not a partner an earlier pair has already cut down. The code fed the
running values forward, so the outcome depended on the order `ANTAGONISTIC_PAIRS`
happens to be written in. Measured: `thermosynthesis` decoded to **0.354 with
`speed` 0 and 0.655 with `speed` 255**, an 85% swing from a gene it is not paired
with at all, because `speed` had already reduced `photosynthesis` in an earlier pair.
All penalties are now computed from a pre-antagonism snapshot. Test:
`antagonism_does_not_depend_on_the_order_the_pairs_are_listed_in`.

#### Note on the harness

The `per_class` block was written, lost to a `sync-lab2.sh` run, and written again,
because the sync rebuilds `src/` from `lab-harness.patch` and the patch had not been
regenerated. **Regenerate and verify the patch in the same breath as any lab change.**

### Step 8 — the rest of section 5, and three things the window showed (2026-09-21)

Everything section 5 still listed is now either fixed or resolved, plus the two
regressions this work introduced and one it exposed. `cargo test` is 238;
`cargo clippy -- -D warnings` and `--all-targets` both pass for the first time.

#### The unread genes (B9) — 11 of 12 implemented

`spec.md` is explicit that **every** gene costs metabolism and that top-N ranks all
46, so "no-op genes take slots and cost energy" is the spec working as written. The
real defect was that twelve genes were declared and never read. Each now does what
`spec.md` says, with a test:

| Gene | What it does now |
|---|---|
| 22 `offspring_scatter` | Children are placed anywhere within the gene's reach (capped by `sense_radius`), not only in the eight adjacent tiles. This is the dispersal mechanism section 4 asked for under founder determinism. |
| 24 `sense_priority` | Splits chemotaxis between food and threat-avoidance. "0 = food, 255 = threats. Gradient." |
| 25 `memory_length` | `place_cell` now writes `memory_dir` on a real move, and `age_memory` expires it after the gene's own tick count. The field was read by `compute_move_target` and written by nothing, so the term was always zero. |
| 28 `adhesion` | Scales down the Move gate in proportion to how much of the neighbourhood is kin. |
| 30 `decay_rate` | A corpse's persistence is now a property of the dead cell. `Tile` gained `decay_fade`, blended by energy when deposits stack, and `fade_decay` uses it. |
| 34/35 `dormancy_*` | A cell below its trigger pays `dormancy_cost` of its metabolism **and takes no action at all**. See the regression below. |
| 41 `territorial_radius` | A non-kin inside the radius is approached rather than avoided. |
| 42 `swarm_signal` | Adds to pheromone emission when the cell is standing on its own kind of food. |
| 43 `gene_linkage` | Mutations run in contiguous blocks. It **redistributes** the load rather than adding to it — a block averages `1/(1-linkage)` bytes and the chance of starting one is divided by the same factor. Without that division a mid-range founder went from ~1.4 mutated bytes per birth to ~6 and B15's heredity went with it. |
| 44 `horizontal_transfer` | A killer may absorb one of its victim's genes. |
| 45 `transposon_rate` | One gene is copied over another inside the genome. |

**`adaptation_rate` (38) is the one exception** and is now listed under Planned
Future Extensions rather than left as a phantom. It is a per-cell modifier layer
that drifts within a life and is not inherited — a feature, not a missing line.

#### G1 — FIXED, then it caused the session's worst regression

The floor promised "every cell has at least one working energy acquisition method"
but was checked against the **raw byte**, before top-N gating. Measured: **1251 of
3000 founders (41.7%) decoded below the floor**. The check now runs on the decoded
value and requires the gene to rank inside the top N.

**Then seed 2 of `default.json` collapsed from a recorded 1377 to 17 at t=10 000.**
The founder census said why: **zero photosynthesizers at tick 0** (thermo 1243,
scav 2177, none 1572). `photosynthesis_rate` is the only acquisition gene paired
with a *non*-acquisition gene (`speed`, "plants don't run"), so whenever a roll
wanted to be a photosynthesizer but rolled high speed, the search quietly made it a
scavenger instead — and scavenging and predation, having no antagonists, always
win that search. Seeding drifted off the one large renewable niche in the world.

The fix is to suppress the chosen gene's antagonists, the same way rival
acquisition genes were already suppressed: the roll decides the *strategy*, and the
genome is adjusted to let it run. Founder photosynthesizers on seed 2: **0 → 923**,
and the run recovers from 17 to **5182 by t=3000**.

Seed 1 is the other way round — 144 at t=10 000 before the fix, 23 at t=3000 after.
Both seeds moved by more than an order of magnitude from a change to founder
composition alone, which is the measure of how much this one function decides.
**The 5-seed 10k table in step 6 needs re-running before any of its numbers are
quoted again.**

**Lesson for the next person: a change to `random_genome` is a change to the
ecology, not to a helper.** Check the tick-0 class census after touching it.

#### Dormancy froze the world — caught from the window, not the harness

The user ran `random_uniform` in the window and reported that cells barely move.
The harness agreed and I had missed it, because I had A/B'd dormancy on
*population* and it looked harmless:

| | dormant | Move | Idle |
|---|---|---|---|
| Dormancy as first implemented | **0.53–0.57** | **0.0–0.3%** | 79–88% |
| After the caps | 0.14–0.23 | 0.1–0.8% | 50–78% |

With `dormancy_trigger` spanning the whole 0–1 range, a random genome sleeps at
60% energy, and over half the population was permanently unconscious — neither
reproducing nor dying. Two knobs fix it: `max_dormancy_trigger` (0.35) makes
dormancy a last resort, and `min_dormancy_cost` (0.25) stops a cheap hibernator
being immortal.

**Population was the wrong metric.** Watch `actions_pct` and `dormant_frac`.

#### Movement: the ceiling is one tile per tick, by spec

`spec.md` gene 6 defines `speed` as "probability of moving each tick", so **no cell
can ever move faster than one tile per tick** however the genes fall. The
hand-designed scavenger and predator decode to 0.56 and 0.63, i.e. they move on
most ticks — they are as mobile as this world allows.

Two things suppress movement below that ceiling, and one is not a gene at all:

- **A packed band cannot move.** `compute_move_target` needs an empty adjacent
  tile. Re-running the archetype bands at `archetype_band_depth` 8 instead of the
  derived 2 took Move from **0.0% to 6.0%** with nothing else changed.
- Photosynthesizers are sessile by selection (`photosynthesis ↔ speed`), so in any
  run they dominate, a grid-wide Move percentage mostly measures *them*.

**Open design question for the user:** if foragers should range further, `speed`
has to mean *distance*, not probability. That is a spec change, not a bug fix, so
it is not made here.

#### Scavengers — four measurements, two fixes, still not solved

The niche is **second-order**: it cannot exist until the producers have turned over
once. Measured `decay_current` at **0.0 for the first ~70 ticks**, the first real
corpse wave 300+ ticks out, against a scavenger runway of `cap / upkeep` = **157
ticks**. Shipped two fixes:

- `initial_decay_matter` (8.0) puts detritus on every tile at world creation.
- `max_scavenge_per_tick` (3.0) — `scavenge_ability` is a rate, not a swallow.
  Uncapped, a maxed scavenger stripped the whole tile on arrival, so
  `corpse_biomass` 25 was one 22-energy meal. One corpse now feeds a cell for
  8+ ticks.

They work, and they are still not enough. On `arch-noPred` the scavenger band goes
**500 → 936 by t=50** on the seeded detritus, then crashes to 0 by t=200 when the
windfall ends — an overshoot, not a starvation. By t=400 `decay_on_occupied`
settles at 3.0, which would pay a scavenger 2.7/tick against an upkeep of 1.2, but
there are none left to take it.

**The energy is there and the timing is wrong.** What is missing is either a
gentler bridge (less detritus, spread over longer) or scavengers seeded *inside*
the producer band rather than beside it, so a corpse is reachable when it appears.

#### The rest of section 5

| Item | Outcome |
|---|---|
| **B16** | **FIXED.** `base_metabolism` is a cost, so the mutual-reduction form meant investing in `sense_radius` or `signal_emission` made a cell *cheaper* — the opposite of `spec.md:190`. Those two pairs are now a one-directional surcharge; capability pairs keep the mutual form. 3 tests. |
| **B13** | **RESOLVED — working as specified.** `spec.md:227` mandates toroidal wrap on both axes and `:250` mandates light entering at `y=0`; the tension is deliberate and is what makes the vertical niche axis an axis rather than a ring. Its one concrete manifestation was the vent zone (B17). The spec now says so explicitly, and says that anything reasoning about depth must measure `y` from the bottom **without** wrapping. |
| **Phase fitness** | **ANSWERED: yes, 1.74x.** Two genomes identical but for one byte — the same slot, the same modifier, one trigger that fires when the boost is worth having and one that fires when it is wasted. **331 ticks vs 577** to reach the same population. Two design traps on the way: run interleaved in one world a control of two *identical* lineages still split 423/609 (holding a column compounds), and measured at a fixed tick both sat at carrying capacity (1032 vs 1030). Separate worlds, growth rate, and a gene with headroom — `apply_modifier` clamps at 1.0, so a maxed gene has nowhere to go. |
| **Decode caching** | **DONE, and the 16.5% figure no longer holds.** `DecodeCache` memoizes per cell per tick, cleared at the top of each tick, invalidated on the two events that rewrite a genome (a recycled id, horizontal transfer). Verified **bit-identical** on 4 configurations via `LAB_NO_CACHE`. Measured **~6%** (5.51s → 5.20s mean over 5 runs), not 16.5%. Replacing the top-N sort with an O(n) `select_nth_unstable_by` was separately bit-identical and measured as **noise**. Do not trust the old profile; it was taken on a different workload. |
| **clippy** | **CLEAN**, both `-D warnings` and `--all-targets`. 54 of the 56 errors were `empty_line_after_doc_comments`, which is exactly the `@veridikt` convention CLAUDE.md mandates — that lint is now `allow`ed at the crate root with the reason written down, and the rest are really fixed. |

#### Harness

`lab-harness.patch` is regenerated and **verified to reproduce the working lab
byte-for-byte**. It now carries the lineage-inheritance fix and `LAB_*` env
overrides for sweeping an archetype's design without a rebuild:
`LAB_PRED_EFF`, `LAB_PRED_ATTACK`, `LAB_PRED_MATURITY`, `LAB_PRED_COOLDOWN`,
`LAB_PREY_ARMOR`, `LAB_PREY_FLEE`, `LAB_PREY_MAXAGE`, plus `LAB_NO_CACHE` and
`LAB_NO_DORMANCY` for A/Bs. The report gained `xtab` (founder lineage x decoded
class) and `dormant_frac`.

**Any change to a file the patch touches breaks its hunks.** Regenerate it in the
same breath, and *verify* — a silent `1 out of 1 hunk FAILED` cost one wrong
measurement this session.

#### Predators, continued from step 7

Sweeping the untried brakes did not find a sustained web, but it did find the shape
of the problem. Prey armour only slides the system between two failures, because
damage changes **how fast** a predator kills, not the fact that each kill funds a
child:

| prey armour | predator attack | predator maturity | Outcome, 3 seeds x 2000 ticks |
|---|---|---|---|
| 6 (baseline) | 180 | 9 t | extinct by t=250 |
| 70 | 180 | 9 t | 2 of 3 extinct; survivor is predator-lineage-turned-photosynthesizer |
| 110 | 180 | 9 t | producers live (3915 photo + 173 vent), **predators starve** |
| 110 | 235 | 9 t | all extinct by t=1000 |
| 110 | 235 | 157 t | all extinct by t=1000 |
| 110 | 235 | **313 t** | **2 of 3 seeds sustain hunters to t=2000** (714 and 404 hunters) |
| 110 | 235 | 392 t | producers live, predators gone |

The one configuration that sustains predators does it in an unexpected way: the
**predator lineage splits**, part of it mutating into photosynthesizers that the
rest then hunt (`PRED->hunter` 714 alongside `PRED->photo` 104). A trophic pair is
sustainable in this world — it just evolves rather than being seeded, and the
seeded producers are dead by then.

The same thing happens on equal shares without any brakes: predators eat everything
by t=50, and by t=2000 their descendants are **676 photosynthesizers with 0.0
attacks per tick**. Selection abandons predation as soon as the prey runs out.

### Step 7 — PresetArchetypes, and what a controlled start measures (2026-09-21)

`SeedStrategy::PresetArchetypes` is implemented. It is no longer an alias for
`RandomClusters`.

**Baseline re-confirmed first.** 5 seeds x 10 000 ticks on `default.json`, matching the
step-6 table exactly: 207 photo / **1377 (photo 1359 + thermo 5 + scav 13)** / 61 thermo /
15 scav / 2938 photo. No extinctions. That run costs about two hours; do the balance work
at 2000 ticks and spend 10k only on a final confirmation.

**Harness bug — `lineage` was never inherited.** `lab-harness.patch` documents
`Cell.lineage` as "inherited by children", but nothing copies it: `Cell::new` defaults it
to 0 and `resolve_reproduction` never sets it. Every newborn in every run of every session
was therefore tagged lineage 0. Consequences for the numbers already recorded here:

- `lineages` / `top_lin` count *surviving founders plus all descendants lumped into
  bucket 0*. "1 lineage" can mean "all founders dead, only descendants left". Treat every
  lineage count in sections 3-4 as unreliable.
- `genetics.same_lineage_attack_frac` is only trustworthy in the first few ticks, before
  births accumulate. B12's before/after (1.00 -> 0.00, measured at t<=10) still stands.

Fixed in the lab copy (`outcome.child.lineage = parent.lineage`) and folded into
`scratchpad/sync-lab2.sh`, which rebuilds the lab from the repo plus the patch plus the
lab-only extras. The patch itself has **not** been regenerated yet — do that before the
next session relies on it.

#### The four archetypes

`spawner::archetype_genome` builds each one lean: one maxed acquisition gene, the support
genes it needs, everything else at byte 6, so `metabolic_cost` stays low and top-N gating
has slots free for the genes that define the species.

| Archetype | Key genes | upkeep | Income in its niche |
|---|---|---|---|
| photosynthesizer | photo 255, speed 0, flee 0, cap 200 | 0.82 | 3.93 full sun / 1.60 at light 104 |
| vent-feeder | thermo 255, speed 0, flee 0, cap 200 | 0.82 | 0.98 at 40 cells sharing one vent (break-even 47.7) |
| scavenger | scavenge 255, speed 150, chemotaxis 220, sense 255 | 1.20 | 22.5 standing on one fresh corpse |
| predator | attack 180, predation 255, speed 170, sense 255, cap 140 | 1.15 | 109 damage, banks 114 per kill = 99 ticks of upkeep |

Two things are deliberate and differ from the random strategies:

- **Phase slots start switched off** (condition `ThreatNearby`, threshold 252, neutral
  modifiers). `spec.md` wants random founder phase bytes under the *random* strategies
  because evolution is supposed to clean the table up. This is the controlled start, and a
  food-web measurement should not also be measuring founder phase noise. Mutation reopens
  the slots over a run. Test: `archetype_phase_slots_start_switched_off`.
- **`aggression_trigger` is set per role.** A neighbour counts as kin when
  `genetic_distance < aggression * (0.5 + 0.5 * precision)`, both read raw (B12). The four
  archetypes sit 0.042-0.125 apart, so the three non-predators get a band of 0.181 (they
  read everything as kin and do not brawl at the borders) and the predator gets 0.050
  (the other three read as prey, its own descendants do not). Test:
  `a_predator_reads_the_other_archetypes_as_prey_but_not_its_own_kin`.

#### Layout: bands, not square clusters — forced by the light column

The first layout was `spec.md`'s literal reading: four square clusters as a 2x2 block
anchored on a vent. It measured **`sun_used` 15.6 of 255** on occupied tiles, photosynthesis
paying **0.24 against an upkeep of 0.82**, and everything dead by t=300.

Two causes, both geometry:

1. Anchoring on a vent puts the block ~85% of the way down the light column.
2. Every occupied tile adds 0.2 to the Beer-Lambert absorption (`world::tile_absorption`),
   so a 51-row-deep producer colony shades itself out: `0.819^50 = 4e-5` of the incident
   light reaches its bottom row.

**A producer colony in this world has to be a thin horizontal film.** That is also what
step 5's rendered frames showed ("one small green colony pressed against the top rows") and
what April called "a blanket at the top of the map".

The shipped layout is four full-width bands stacked around the wrap seam, each sized so
its own founders fill about half its tiles (`archetype_band_depth`, 0 = derive):

```text
  y = 0            photosynthesizer   brightest rows, nothing above to shade them
  below it         predator           in contact with the producers it eats
  below that       scavenger          in contact with where the predator kills
  ...
  y = H - depth    vent-feeder        the vent row, adjacent to the photic band
```

`spec.md` and `architecture.md` were updated. This is a documented-design change, not a
code/doc reconciliation: the spec said "predefined cluster centers, one per archetype" and
now says one band per archetype. The measurement above is the reason.

#### B17 (new, FIXED) — the vent zone wrapped in Y into the photic rows

`docs/spec.md`, vertical resource axis: "**Top zone:** high sunlight, **no vents**". Both
`tick.rs`'s vent-income loop (`for dy in -r..=r` through `world.wrap`) and
`actions.rs::is_near_vent` wrapped `dy`, so a vent on row `H-1` also reached rows
`0..vent_radius` — the brightest rows, where the Beer-Lambert column starts. Sunlight does
not wrap (`recompute_sunlight` restarts each column at `y = 0`); the vents did.

This was invisible until `vent_radius` went from 1 to 6 in step 6, and invisible again
until an archetype band was deliberately placed at `y = 0`: about **78 photosynthesizers
per vent** then stood inside the zone, dividing `vent_output` without drawing on it.
`adjacent_count` went from ~65 to ~143, so each vent-feeder's share fell from 0.60 to
0.27 against an upkeep of 0.82 and **vent-feeders went extinct even with no predators in
the world**.

Fixed: one definition, `World::is_in_vent_zone`, toroidal in x (the bottom edge is a ring)
and bounded in y. Test `world.rs::the_vent_zone_does_not_wrap_into_the_photic_rows` fails
on the old code ("row 0 is in the vent zone").

Measured, 3 seeds x 2000 ticks, shares 0.5 photo / 0.5 vent-feeder:

| | photosynthesizers | vent-feeders |
|---|---|---|
| Before | 4316-4371 | **0** |
| After | 4275-4420 | **134-223**, stable |

#### Predators: the hypothesis in section 5 was backwards

Section 5 asked whether hunters "can persist at all when prey is placed next to them",
on the theory that prey density collapses before a hunter finds anything. With prey
literally in contact, the opposite happens.

Equal shares (1250 each), seed 1:

| t | photo | vent | scav | predator | kills/tick |
|---|---|---|---|---|---|
| 0 | 1250 | 1250 | 1250 | 1250 | — |
| 25 | **0** | 308 | 628 | **3459** | 205.7 |
| 50 | 0 | **0** | 551 | 3293 | 25.2 |
| 100 | 0 | 0 | 83 | 23 | 1.1 |
| 250 | — | — | — | — | world empty |

The predator eats both producer bands in 50 ticks and then starves. Its numerical response
is about 10x the producers': it matures in **9 ticks** against their 94, and one kill
(~109 energy absorbed) clears its 67-energy reproduction threshold outright, so one kill is
one child. A producer needs ~20 ticks of photosynthesis plus a 94-tick maturity to make one.

**A 2% predator share (100 founders against 4900) ends the same way**, extinct by t=250.

Sweep, 3 seeds x 2000 ticks, shares 0.60/0.20/0.15/0.05, varying the predator's own genes:

| predation_efficiency byte | maturity_age byte (ticks) | Outcome at t=2000 |
|---|---|---|
| 255 (1.00) | 24 (9) | **extinct by t=250** |
| 100 (0.39) | 24 (9) | **extinct by t=250** |
| 40 (0.16) | 24 (9) | 3665-4118 photo + 82-180 vent-feeder, **no predators** |
| 255 | 120 (470) | 2911-3639 alive: photo only, **no predators** |
| 100 | 120 (470) | 2964-3735 alive: photo only, **no predators** |
| 40 | 120 (470) | 3557-4010 photo + 90-143 vent-feeder, **no predators** |

**Maturity is the lever, not efficiency.** Delaying the predator's first birth to 470 ticks
is enough to stop the collapse, but in every surviving configuration the predators
themselves are gone by t=2000. No setting measured so far has predators persisting
alongside prey. Prey in these runs cannot flee (`flee_response` 0) and carry baseline
armour; both are archetype design choices and both are untried brakes.

#### Scavengers: the niche does not exist yet at t=0

With no predators, the scavenger band (500 founders) is gone before t=250, and the trace
says why: **`deaths` is 0.0 and `decay_current` is 0.0 for the first ~70 ticks.** Nothing
has died yet. Producers die of old age at `max_age` 96 -> a 1316-tick lifespan, so the
first corpse wave is ~1300 ticks away; a scavenger's runway is its cap over its upkeep,
188 / 1.20 = **157 ticks**.

The scavenger niche is second-order: it cannot exist until the producer population has
turned over once, or until something kills producers early. That "something" is the
predator — in the equal-share run scavengers outlived the producers. So the scavenger and
predator problems are one problem, and the predator is the one to solve.

#### Where this leaves the question

"Is a food web sustainable here?" — as measured, **no, not yet**, and the reasons are now
specific rather than diffuse:

- Two producers coexist stably (photo ~4300 + vent-feeder ~180 at t=2000, 3 seeds), which
  random seeding reached on 1 seed in 5. That part works.
- The scavenger cannot bootstrap without a corpse supply.
- The predator has no measured setting where it persists without removing its own food.

#### Still open after this step

- **Untried predator brakes:** producer `armor` (not antagonistic with photosynthesis when
  speed is 0, so a sessile producer can be tough for ~0.03 of upkeep; at byte 70 it absorbs
  70 of the predator's 109 damage and a kill takes 4 ticks instead of 1), producer
  `flee_response`, and predator `reproduction_cooldown`.
- **Regenerate `lab-harness.patch`** against this tree, including the lineage fix.
- Everything section 5 still lists: B9/G1, B16, B13, phase *fitness*, decode caching.

### Step 6 in progress — food-seeking and niche access (2026-09-21)

**Food-seeking (was "still open in movement").** `docs/spec.md` gene 9 defines
`chemotaxis_strength` as "tendency to move toward nearby energy sources", but the code
spent that gene on the *pheromone* gradient while `sense.nearest_food` was computed and
never read — nothing in the simulation ever moved toward food. Now chemotaxis steers
toward `nearest_food`, pheromone is its own term weighted by `signal_sensitivity` (which
`sense` already applies), and a new `add_unit` helper normalises all three direction
terms so a food tile four steps away cannot outvote a neighbour one step away.
A predator's "food" is now the nearest prey, not a tile — tile-based detection gave
hunters no target at all, which is April's "predators roam aimlessly even with prey a few
pixels away". Tests: `chemotaxis_steers_toward_food`, `a_predator_hunts_the_nearest_prey`.

**Measured effect on its own: none.** Seeds 1–3 finished within noise of the step-5
numbers and seed 3 still died. Correct per spec, and a prerequisite for predators, but it
does not move the ecology by itself — the niches were not merely hard to find.

**Why "photosynthesizers win" — measured, and it is the environment, not the code.**
A new `--niche` lab mode sizes each niche against a lean specialist's upkeep (0.24):

| Niche | Reachable tiles | Energy/tick | Cells it can feed |
|---|---|---|---|
| Photosynthesis | **262 144** (the whole grid) | 661 432 | ~2.8 million |
| Thermal vents | 45 | 40 | ~168 |

Two separate causes, both of them config or seeding rather than sim logic:

1. **There is no dark zone.** At the default `sunlight_gradient_strength = 1.0` on a
   512-row grid the per-row absorption is `1.0/512`, so the *bottom row still receives
   94/255 light* — enough for photosynthesis to pay everywhere. `docs/spec.md`'s three
   zones (top photic, middle scarce, bottom vents) do not exist. Bottom-row light by
   gradient: 1.0 → 93.8, 3.0 → 12.7, 5.0 → 1.7, 7.0 → 0.2.
2. **No cell could ever reach a vent.** Measured with a new `vents` probe: the nearest
   living cell sat **49 tiles** from the closest vent, and `cells_adjacent` was **0** in
   every run of the whole session. Two documented claims were false in the code:
   - `spec.md` says cluster centres are distributed across the full Y axis and "some
     clusters land near thermal vents at the bottom". They were centred inside their own
     band, leaving the bottom row of clusters ~49 tiles short. Rows now span the full
     axis, edges included (`spawner.rs::cluster_rows_reach_the_vents`).
   - `spec.md` describes a bottom *zone* fed by vents, but a vent fed a 3x3 patch —
     nine tiles in 262 144. New `config.vent_radius` (default 6) makes the vent's output
     reach a zone, still shared among everyone in it.

**First result:** on seed 3 — the seed with *no photosynthesiser founders*, which has
died in every run this session — a thermo colony now survives on the vents at 1.43
energy/cell-tick. Small (8 cells, the 5x8 vent budget only feeds ~23) but it is the first
non-photosynthetic survivor of the session.

**Knob sweep, 36 runs (3 gradients x 2 vent counts x 2 outputs x 3 seeds, 2000 ticks):**

| Knob | Effect |
|---|---|
| `vent_output` 8 → 40 | Works now: seed 3's thermo colony grows 7→43, 8→40, 5→56 cells. Before `vent_radius` this knob did **nothing** |
| `vent_count` 5 → 12 | Scavengers appear on seed 2 (up to 15 cells) |
| `sunlight_gradient_strength` 1 → 3 → 5 | **Almost no effect.** My reasoning above was wrong: the gradient changes light at the *bottom*, but every survivor lives in the top rows where light is ~255 regardless. It would only matter if photosynthesisers colonised the bottom, and they never get that far |

Defaults adopted from the sweep: `vent_count` 12, `vent_output` 40. Left
`sunlight_gradient_strength` at 1.0 since it measurably does not matter.

**10 000 ticks, 5 seeds, adopted config:**

| Seed | t=10 000 | Classes |
|---|---|---|
| 1 | 207, growing | photo |
| 2 | **1377, growing** | **photo 1359 + thermo 5 + scav 13** |
| 3 | 61, stable | thermo (vent colony) |
| 4 | 15, stable | vent dwellers |
| 5 | 2938, growing | photo |

**No extinction on any seed at 10k ticks**, against "extinct by tick 30 on every seed" at
the start of this work. Section 6's target is met on **1 of 5 seeds** (three coexisting
classes); the other four end as single-class worlds.

**The remaining obstacle is founder determinism, not selection.** Each seed ends as
whatever its founders were: seed 3 had no photosynthesiser founders and ends thermo-only;
seed 5's scavenger founders died early and it ends photo-only. Clusters sit 128 tiles
apart, cells barely move, and one mutated byte per birth cannot invent a new strategy, so
a cluster either survives in place or dies — they never meet and no trophic web assembles.
**Predators never appear on any seed**, even seed 2 which started with 312 hunter
founders: predation pays now (one kill of a 50-energy prey covers 77 ticks of upkeep) but
prey density collapses in the first ~100 ticks, before a hunter can find anything.

**Recommended next step:** implement `SeedStrategy::PresetArchetypes`, which `docs/spec.md`
already specifies ("3-4 hand-designed species ... with mutations. Controlled start") and
which is currently reserved and unimplemented. Placing photo / scavenger / predator
clusters *adjacent* answers the question that random seeding cannot: is a food web
**sustainable** here, separate from whether it randomly assembles. If it is, the remaining
work is migration and seeding, not mechanics.

### Performance: the tuning loop was the bottleneck (2026-09-21)

A 2000-tick run cost ~5 minutes, which made every balance sweep painful. I first blamed
B10 (`cell_ids` scanning the free list) **without measuring, and was wrong** — `perf` does
not show `cell_ids` at all; LLVM vectorises that scan. The real profile:

| Cost | Share |
|---|---|
| `diffusion::diffuse_layer` | **43.4%** |
| top-N gating sort inside `decode` | 16.5% |
| `run_tick` | 6.8% |
| `run_diffusion_phase` | 5.5% |
| `powf` (metabolic cost) | 5.0% |
| `sense` | 3.4% |

Diffusion sweeps all 262 144 tiles x 3 layers **every tick regardless of population**, so a
run costs the same with 100 cells alive as with 5000 — which is why runs stayed slow long
after the die-off. `diffuse_layer` was doing 8 `rem_euclid` integer divisions per tile per
layer (~6.3M per tick) when only the border needs wrapping. It now uses plain index
offsets for interior tiles, in the same accumulation order.

**Measured on an idle machine: 8.1–8.8 s → 3.6 s per 300 ticks, a 2.2x speedup**, with
output byte-identical on seeds 1–3 (determinism preserved). `cell_ids` was also cleaned up
(the free-list scan is redundant — `kill_cell` zeroes energy first) but that one is tidiness,
not speed.

Still unoptimised if more is needed: `decode` runs ~7x per cell per tick and re-sorts 46
genes each time (16.5%). Caching it per cell per tick is the obvious next win.

**Benchmark on an idle machine.** A first A/B under a 12-way parallel sweep reported the
faster build as *slower*.

### The window runs again (2026-09-21)

`main.rs` is no longer a stub. It loads a JSON config (first argument, else
`WorldConfig::default()`), spawns the sim thread, and runs a `winit` 0.30 +
`pixels` 0.14 window fed by an `ArcSwap<WorldSnapshot>` — the threading model
`docs/architecture.md` specifies. This replaces the lost April `feat/main-loop`
branch.

- **Keys:** space pause · `1` genetic · `2` strategy · `3` phase · `4` energy ·
  ↑/↓ speed (ticks per published frame, 1–1024 — April's fast-forward, done by
  skipping *frames* rather than throttling the sim) · `r` reseed · esc quit.
- The title bar carries tick, population, speed and mode, so "is anything
  alive?" is answerable at a glance.
- **Build note:** `pixels` 0.14 still uses `raw-window-handle` 0.5 while `winit`
  0.30 defaults to 0.6, so `Cargo.toml` enables winit's `rwh_05` feature. Without
  it the two crates do not compose at all.

**Verifying without a compositor.** The machine is on Wayland and `import` can
only capture X surfaces, so the lab gained `--render <prefix> --shots a,b,c`,
which runs the sim headless and writes the *renderer's own framebuffer* to PPM
for each colour mode. That is the same `Renderer::render` the window calls, so
what it produces is what the window shows.

What the frames show on seed 1 (`default.json`):

- **t=10, genetic:** 16 founder clusters on the 4×4 layout, each a distinct
  colour family — strategy hue plus lineage drift, exactly what B8 was supposed
  to deliver.
- **t=10, phase:** every cluster is a speckled mix of cyan/yellow/magenta/grey —
  founder phase noise, P1 made visible.
- **t=300 → t=1500, genetic:** one small green colony pressed against the top
  rows, everything else gone. This *is* April's "blanket at the top of the map",
  and it is the niche problem rendered: photosynthesis is the only niche a cell
  reaches without moving.
- **t=1500, phase:** the survivors are uniformly yellow — one evolved phase slot,
  no longer noise.

### Step 5 fixes applied — visibility (2026-09-21)

| Fix | Change | Test |
|---|---|---|
| B14 | `WorldSnapshot.cells` is now `Vec<CellView>` instead of `Vec<(u16, u16, GenomeHash)>`. A `CellView` carries position, genome hash, **strategy** (the dominant acquisition channel), **specialization** (gap to the runner-up), **energy_fraction** (share of the cell's own cap), **age_fraction** (share of its own lifespan) and **active_phase**. `World::snapshot` takes the config so it can decode; `Simulation::snapshot` passes it. | `world.rs::snapshot_carries_what_the_renderer_needs`, `::snapshot_classifies_an_unexpressed_genome_as_none` |
| B8 | `render::color::cell_to_rgba(&CellView, ColorMode)` replaces `genome_hash_to_rgba`. Hue band from strategy (photo green / thermo ember / scavenge ochre / predation magenta / none slate), ±22° of drift within the band from the genome hash, saturation from specialization, brightness from energy. | `color.rs::same_strategy_stays_in_one_hue_band`, `::different_strategies_are_far_apart`, `::energy_shows_as_brightness` |

`ColorMode` gives the debug views April asked for ("good for debugging but too
restrictive... for regular use a hybrid of this method and the previous one"):
`Genetic` (default hybrid), `Strategy` (flat per class), `Phase` (flat per slot — this is
the one that answers "did phases evolve?") and `Energy` (heat ramp). `Renderer::set_mode`
switches at runtime.

**Colour separation, measured with the new `--color-check` lab mode** (2000 genome pairs,
the same metric the original B8 finding used):

| | Old hash mapping | Now |
|---|---|---|
| Mean hue shift, one byte changed | 87.6° | **14.6°** |
| Mean hue shift, unrelated genomes | 90.0° | **77.2°** |
| Siblings inside one hue band | — | **100%** |

A lineage is now visually trackable and a strategy is readable at a glance, which is what
"mixed-color colonies" was really about. `docs/spec.md` (Visual Representation) and
`docs/architecture.md` (Rendering Pipeline, snapshot shape) were updated — the old text
claimed a genome *hash* made similar cells look similar, which was never true.

**Not done in step 5:** the window loop. `main.rs` still only prints the config, and
nothing uses `winit`/`pixels`/`arc-swap`. The renderer is a pure
`WorldSnapshot -> Vec<u8>` projection that is ready for it.

### Step 4 fixes applied — phases (2026-09-21)

| Fix | Change | Test |
|---|---|---|
| B7 | New `actions::effective_genes` (decode + `apply_phase_modifiers` for the cell's `active_phase`) replaces the bare `decode` at every site in `resolve_all`: reproduction, both sides of combat, movement conflicts and share. `tick.rs` vent income does the same. The offense and defense phase groups previously had **no effect on anything**. | `actions.rs::phase_offense_modifier_reaches_combat` — on the old code it reports identical damage, `200 vs 200` |

**P1 is not a bug.** `spec.md` is explicit that founder phase bytes are random on purpose
("Most newly-spawned cells will have nonsensical phase triggers. That is intentional --
evolution cleans up the phase table over many generations"). Since B15 gave lineages real
heredity, that is now what happens, and the measurement is the interesting part:

| | Founders (t=5–10) | Evolved (t=1500) |
|---|---|---|
| Non-default phase | 69–83% | 58–92% |
| Trigger conditions in use | 6–8, spread (seed 1: no_food 1391, energy_high 652, kin 438, age 308, crowded 292) | **one dominates**: seed 1 `energy_low` 65 of 69 active (94%), seed 2 `age_mature` 269 of 301 (89%) |
| Phase transitions per tick | 19–20 | **2.0–2.8** |
| Modifier deviation from neutral | — | 26–50 points off 128, so the surviving phases carry real modifiers |

Founders flicker between random triggers; survivors hold one selected trigger with a
stable modifier set. That is the "phase occupancy differs from founder noise" criterion
from section 6, met on two seeds.

**Hysteresis (section 5, hypothesis 4) — resolved, the code is right and the spec was
wrong.** `evaluate_phase` enters at `value >= threshold` and stays until the value falls
below `threshold * (1 - band)`, i.e. sticky. The spec said "exit requires crossing
threshold + band", which reads backwards for conditions normalised so higher = stronger.
`docs/spec.md` has been corrected, along with the claim that a slot is disabled "by
setting trigger_threshold to 0 or 255 depending on condition polarity" — a threshold of 0
fires *always*; only a high threshold disables a slot.

Still open in phases: whether a phase slot confers measurable *fitness* (two genomes
identical but for one slot) is not yet tested.

### Step 3 fixes applied — heredity, trophic links, lifecycle (2026-09-21)

| Fix | Change | Test |
|---|---|---|
| B15 | `mutate` takes the config: the rate byte scales within `max_mutation_rate` (0.05/byte) and the magnitude byte within `max_mutation_magnitude` (24). Meta-evolution survives — a fast mutator still mutates far more than a slow one — but no founder rewrites half its genome per birth. | `genome.rs::mutate_changes_only_a_few_bytes_per_birth`, `::mutation_genes_still_scale_within_the_bound` |
| B3 | A kill now feeds the killer: the attacker absorbs `predation_efficiency ×` the victim's energy as it stood before the blow. Nothing is written back to the corpse — `is_alive` is `energy > 0`, so that would resurrect it. | `actions.rs::killing_feeds_the_killer`, `::surviving_an_attack_feeds_nobody` |
| B12 | `sense` reads `aggression_trigger` and `kin_recognition_precision` **raw from the genome** instead of from the top-N-gated decoded values. Recognition is a threshold, not an expressed capability, and gating pushed it to ~0.05 so siblings at distance 0.13 read as threats. | `actions.rs::kin_recognition_survives_top_n_gating` |
| B4 | Corpse deposit is `corpse_biomass` (25) + `corpse_energy_fraction` (0.5) of energy left, replacing `abs(energy) * 0.5` — which paid out combat overkill as a bonus and left starved cells with nothing. | `tick.rs::starved_corpse_still_leaves_biomass` |

**Two more fixes the harness forced, both closing spec gaps rather than inventing behavior:**

| Fix | Why |
|---|---|
| **Senescence** (`max_age`, gene 32, `spec.md:85` "tick count before natural death") was never implemented. With the economy working, colonies reached energy equilibrium and simply *froze*: at t=500 on seed 1 every cell was an original founder, `mean_age` 500, births 0, deaths 0, energy pinned at the cap. That is April's "persistent colonies that just don't seem to do anything". `energy::lifespan_ticks` maps the gene between `min_lifespan_ticks` (300) and `max_lifespan_ticks` (3000), checked in `update_energy`. |
| **Maturity capped by lifespan.** Senescence alone then killed seed 2 outright. The new `repro_gate` probe showed why: `sterile_by_design: 117` — every cell's maturity (937 ticks) exceeded its own lifespan (369), so the lineage could never reproduce at all. `mapped_maturity_age` now clamps to `maturity_lifespan_fraction` (0.5) of the cell's lifespan, the same kind of physical cap as `attack_range <= sense_radius`. Added to the spec's Physical Caps list. |

Also: the reproduction gate is `>=`, not `>`. Energy is clipped at the storage cap, so a
cell whose threshold gene sits at 1.0 could never qualify.

**Harness, `default.json`, seeds 1–3, 1500 ticks:**

| Seed | After step 2 | After step 3 | Notes |
|---|---|---|---|
| 1 | 333 @1000, 1 lineage | **112 @1500**, 2 lineages | births 3.2 ≈ deaths 3.2, scavenging appears (0.38/cell-tick) |
| 2 | 8 @1000, frozen | **307 @1500**, 1 lineage | was extinct @600 with senescence alone; the maturity cap rescued it |
| 3 | extinct @250 | extinct @100 | see below — its founders contain **no photosynthesizers at all** |

| Metric | Before step 3 | After |
|---|---|---|
| Mutated bytes per birth | 35–58 / 64 | **1.0–1.4** |
| Parent→child distance | 0.07–0.36 (strangers ≈0.33) | **0.0003–0.0006** |
| Same-lineage attack fraction | 1.00 | **0.00** |
| Attacks per tick (early) | 620–710 | **0.0–0.1** |

**Seed 3 is the clearest statement yet of the remaining problem.** Its founders are
thermo 624 / scav 940 / none 3428, with zero photosynthesizers, and it dies by t=100.
Measured income at that point: photo 0.013, **thermo 0.000**, scav 0.017, against
metabolism 1.95. Vents are 5 × 9 = **45 tiles out of 262 144**, so the thermal niche is
statistically unreachable by a random cluster, and scavengers cannot find corpses
because nothing steers a cell toward food (`sense.nearest_food` is computed and never
used; `chemotaxis_strength` follows pheromone instead, contrary to `spec.md:37`).
Photosynthesis is not winning because it is strong — it is the only niche a cell can
reach by standing still. That is the target for step 6, and the food-seeking gap should
be fixed before any niche re-sizing is attempted.

### Step 2 fixes applied — energy economy (2026-09-20)

Measured first with a new lab mode, `--archetypes`, which prints income vs upkeep for
hand-built specialists. That is the feasibility check the balance work needs, and it is
also the tool section 5's niche-capacity hypothesis asks for.

**Before (salvaged code), hand-built lean specialists:**

| Archetype | upkeep | best income | net | ticks to reproduce |
|---|---|---|---|---|
| photo | 3.23 | 3.91 full sun / 1.96 half | +0.68 / **−1.27** | 223 |
| thermo | 3.23 | 7.82 alone / 0.87 with 9 on a vent | +4.59 / **−2.36** | 33 |
| scav | 3.63 | 9.00 at decay 10 | +5.37 | 28 |
| hunter | 3.24 | 0.28 | **−2.96** | never |

Most of that 3.2 upkeep came from *policy* genes (reproduction threshold, offspring
share, temperature preference) and the storage cap — not from any capability.

| Fix | Change | Test |
|---|---|---|
| B5 | New `config.metabolic_cost_scale` (default 0.2) multiplies the summed expression cost. The spec's invariant still holds: an all-max genome pays 9.2 against a maximum income of 4.0, while a lean specialist pays under 1.0. | `energy.rs::lean_specialist_earns_more_than_it_spends` |
| B6a | `energy::storage_cap` is now the single definition of capacity: `energy_cap_floor` (60) + gene share up to `energy_cap_max` (255). Replaces three copies of `gene * 255.0`. | `energy.rs::storage_cap_has_a_floor_and_a_ceiling` |
| B6b | `starting_energy` is clamped to the genome's own cap — the excess was destroyed on tick 1 anyway. | `spawner.rs::spawn_energy_never_exceeds_storage_cap` |
| B6c | Reproduction needs `max(threshold_gene * cap, config.reproduction_energy_floor)` (40), so a tiny-cap cell can no longer split at near-zero energy. | `actions.rs::reproduction_threshold_respects_absolute_floor` |

Also moved to config: `photo_max_income` (was a hardcoded 4.0) and `scavenge_efficiency`
(was 0.9). `docs/spec.md` and `docs/architecture.md` were updated for all of it.

**After, same archetypes:** photo +2.94 net (54 ticks to reproduce), thermo +6.86,
scav +7.91, hunter still **−0.69** (B3 — predation pays nothing).

**Harness, `default.json`, seeds 1–3, 1000 ticks:**

| Seed | Before (t) | pop @250 | @500 | @1000 | lineages | income vs upkeep @1000 |
|---|---|---|---|---|---|---|
| 1 | extinct @30 | 131 | 159 | **333** | 2 | photo 2.05 vs metab 1.94 |
| 2 | extinct @30 | 9 | 8 | **8** | 2 | photo 1.98 vs metab 1.99 |
| 3 | extinct @30 | 10 | 21 | **222** | 1 | photo 1.60 vs metab 1.39 |

No extinctions, births ≈ deaths with real turnover, `cap_waste` down from ≈36 to
0.05–0.27, `pos_mismatch` and `ghosts` 0 throughout. Mean metabolism *falls* over a run
(1.94 → 1.39 on seed 3), so selection is already trimming expression cost.

**The endpoint is still "photosynthesizers win":** every survivor is photo, 1–2
lineages. That is expected — B3, B12 and B15 are all step 3 — but it means step 2 is
verified for *feasibility*, not for balance. Do not tune further until step 3 lands.

Knob sweep behind the defaults (3 seeds × 300 ticks): `metabolic_cost_scale` 0.3 → slow
bleed-out, 0.25 → marginal, 0.2 → stable growth. `reproduction_energy_floor` 40 gave
roughly 2–3× the births of 60 at the same scale.

### Step 1 fixes applied (2026-09-20)

| Fix | Change | Test | Measurement |
|---|---|---|---|
| B1 | `World::clear_next` → `World::prepare_next`: copies every env field current→next with `cell_id = 0`, called from `run_tick` right after `update_sunlight` instead of after the swap. | `world.rs::prepare_next_carries_environment_and_clears_cells` | `sun_used` 0.0 → equals `sun_current`; `decay_used` equals `decay_current` |
| B2 | `place_cell` now writes `cell.position` alongside `tile.cell_id`, so tile and field can never diverge. | `actions.rs::resolve_all_move_updates_position_for_next_tick` (moves, swaps, then idles — the cell must not snap back) | `pos_mismatch` ≤9 → 0 |
| B11 | **Was real.** A Move winner was checked only against `current`, so it overwrote a newborn already placed in `next`, leaving the child alive in the pool on no tile. Winners whose target is already taken in `next` now stay at their source. `place_cell` also carries a `debug_assert` that fires on any overwrite. | `actions.rs::resolve_all_move_does_not_overwrite_newborn` (fails on the old code) | `ghosts` 0 across all runs; the assert is silent through the whole suite |
| Heading bias | `direction_noise` was one-sided (`rng * noise * π`), rotating every heading the same way — constant curvature, i.e. the arcs and drifting flocks from April. Now symmetric: `(rng*2−1) * noise * π`. | `actions.rs::direction_noise_is_symmetric` | Old code: CW/CCW tally **2022 vs 0** (skew 1.0). Fixed: skew < 0.1 |
| Toroidal headings | `compute_move_target` scored candidates with the raw `tx − cx`, so at the seam one neighbor scored ±(width−1) and dominated; `pack_affinity` and `flee_direction` had the same flaw. All three now use a new `toroidal_delta`. | `move_target_uses_toroidal_delta_at_seam`, `flee_crosses_seam_away_from_threat`, `toroidal_delta_takes_short_way` | Old code sends an edge cell heading −x to `(0,4)` instead of across the seam to `(15,5)` |

Harness `move_bias.mean_err` over seeds 1–3 (512², t≤20): **0.465 / 0.691 / 0.535 → 0.235 / 0.585 / 0.086**. Treat that as weak evidence only: at n = 119–408 moves (populations die by t≈25) it is noisy, and the residual mixes in 8-direction discretization plus the pheromone and pack terms. Re-measure once cells survive; the unit tests are the real guard.

**New finding — B16 (`base_metabolism` antagonism is inverted).** `docs/spec.md:190,192`
says the `sense_radius` ↔ `base_metabolism` and `signal_emission` ↔ `base_metabolism`
pairs exist because "awareness increases metabolic drain" and "broadcasting is
energetically expensive". But `apply_antagonistic_pairs` *reduces* the effective value
of both genes in a pair, and `base_metabolism` is a cost ("lower is more efficient",
`spec.md:28`). So investing in sensing or signalling currently makes a cell **cheaper**,
the opposite of the documented intent. Impact today is small (the gene is just one term
in the cost sum), but it becomes real if metabolism is ever restructured as
"base + expression" the way `architecture.md:189` describes. Not fixed.

**Still open in movement (not yet fixed):**

- `sense.nearest_food` is computed and never used. `chemotaxis_strength` steers by *pheromone*, while `docs/spec.md:37` defines it as "tendency to move toward nearby energy sources". No cell ever steers toward food, which is likely a large part of "predators roam aimlessly even with prey a few pixels away".
- `Cell.memory_dir` is still never written (`memory_length` is a no-op gene, B9).
- `direction_bias` is an absolute per-genome heading, so a cluster of near-identical genomes walks one fixed direction forever. That matches the spec, but combined with no food-seeking it is what makes colonies march off and die.
- The best-tile tie-break still falls to `empty_adjacent[0]` (top-left). Ties are rare with a continuous heading; unmeasured.

**Not a bug:** `resolve_movement_conflicts` iterates a `HashMap`, but its writes go to disjoint tiles (winners to distinct empty targets, losers to their own occupied sources), so the outcome is order-independent and the determinism test passes.

## 5. Open hypotheses for the next session

Everything this section used to list is closed. What follows is what is actually
left, in the order I would take it.

1. **Scavengers in the archetype bands still cannot bootstrap.** Under *random*
   seeding this is largely resolved by step 10 — scavengers now persist to 10k
   (766 on uniform seed 2, 1992 on clusters seed 3). The hand-built band still
   goes 500 → 936 → 0 by t=200 (measured before step 10): an overshoot on the
   seeded windfall. Revisit after the archetypes are re-tuned (item 3).

2. **No seeded four-way food web is sustainable yet.** The only configuration that
   keeps hunters alive to t=2000 does it by the predator lineage splitting into
   producers and hunters; the seeded producers are dead by then. Full sweep table
   in step 8.

3. ~~**Mobility is structurally unreachable for a random genome**~~ — **fixed in
   step 10** by moving top-N gating ahead of the antagonistic pairs. Three or more
   classes now coexist on 2 of 3 seeds at 10k under uniform seeding, with hunters
   carrying positive net. `random_clusters` re-measured at 10k: two seeds improve
   13x and 55x with mobile classes, **seed 2 collapses to 22**. **The archetype bands
   broke** (equal shares extinct, pyramid ends all `PRED->photo`): their genomes were
   tuned against the old decode order and need re-tuning. `--niche` and
   `--archetypes` not yet re-run.

4. **A packed region cannot move at all** — `compute_move_target` needs an empty
   adjacent tile. Measured: Move 0.0% → 6.0% purely from making the archetype bands
   sparser. Worth deciding whether dense colonies *should* be immobile.

5. **`adaptation_rate` (gene 38)** is the one gene the simulation does not read. Now
   listed under Planned Future Extensions in `spec.md` rather than left as a phantom.

6. ~~**Diffusion still costs the same with 18 cells alive as with 5000.**~~ **Done in
   step 11**: 25.2 → 5.7 ms/tick on `default.json`, and the cost now scales with the
   population. Also found and fixed the temperature map being erased by tick 300.

7. **Not yet re-measured after step 8:** the `--niche`, `--color-check` and `--render`
   modes, and the 5-seed 10k table in step 6. The economy moved under all of them.

## 6. Suggested fix order

Balance tuning is meaningless before steps 1–3 are done.

1. ~~**Tick integrity**~~ — **done 2026-09-20.** B1, B2, B11 and the two movement-heading bugs are fixed with tests; see "Step 1 fixes applied" in section 4.
2. ~~**Economy feasibility**~~ — **done 2026-09-20** for B5 and B6; see "Step 2 fixes applied". B9 was **not** done: no-op genes still cost metabolism and still take top-N slots (see G1). With the cost scale at 0.2 their contribution is ≈0.03 each after gating, so it is no longer the blocker it was — but the top-N displacement is untouched.

   Original notes:
   - Charge metabolism only for "capability" genes, or remove no-op genes from cost and from top-N ranking.
   - Clamp spawn energy to the cap.
   - Derive the reproduction threshold from absolute energy, not `gene * cap`.
   - Re-derive `PHOTO_MAX_INCOME` against real costs.
3. ~~**Heredity and trophic links**~~ — **done 2026-09-21.** B15, B3, B12, B4, plus senescence and the maturity cap. See "Step 3 fixes applied".
4. ~~**Phases**~~ — **done 2026-09-21.** B7 fixed; P1 closed as working-as-specified once heredity worked (do *not* force founder slots off — the spec wants them random). See "Step 4 fixes applied".
5. ~~**Visibility**~~ — **done 2026-09-21** for B14 and B8; see "Step 5 fixes applied". **The window loop is still not built** — that is the next piece of work, and everything it needs (snapshot shape, colour modes) is now in place.
6. Only then balance niche capacities, using the harness across ≥5 seeds and ≥10k ticks. Target: ≥3 strategy classes coexisting, and phase occupancy that differs from founder noise.
8. ~~**Everything in section 5**~~ — **done 2026-09-21**, see "Step 8": B9 (11 of 12 genes), G1, B16, B13, phase fitness, decode caching, clippy, and the harness patch. Two regressions this work introduced were caught and fixed (G1 wiping photosynthesizers from seeding; dormancy freezing half the population); the scavenger and predator questions are still open and are now section 5.
7. ~~**PresetArchetypes**~~ — **done 2026-09-21.** Implemented with bands, plus B17 (vent zone wrapped into the photic rows). See "Step 7". The target is still unmet: two producers coexist, the scavenger has no corpse supply at t=0, and the predator has no measured setting where it persists without eating its own food out.

## 7. Rules to keep

- Determinism: same seed + config must be bit-identical. Use only the `ChaCha8Rng` passed down; no `thread_rng`, no `HashMap`-order dependence, no parallel float reductions.
- Keep `@veridikt` annotations in sync when behavior changes (lower priority than code, per the user).
- Never commit; the user does git.
- Balance knobs belong in `WorldConfig`.
