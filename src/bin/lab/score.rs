//! Score mode: the benchmark. Runs a config over several seeds and scores
//! each run from its time series over a window, not from one snapshot.
//!
//! The question it answers is not "who is most numerous" — producers
//! outnumbering consumers is the normal trophic pyramid — but "does every
//! way of living persist", plus how thick the band of life is and how much
//! of the population is in an evolved phase.
//!
//! With `--vs-set key=value` (repeatable) or `--vs file.json`, every seed is
//! also run under a second arm and the report gives paired differences. The
//! same seed gives both arms identical founders whenever the change leaves
//! seeding alone, so pairing cancels the founder lottery, which is the
//! largest source of variance between runs.

use std::io::Write;

use serde_json::{Value, json};

use primordium::config::WorldConfig;
use primordium::sim::Simulation;

use crate::census::{self, CLASSES, Census, GROUPS};
use crate::cli::{
    Args, apply_overrides, apply_sets, die, mean_sd, parse_seeds, r3, read_config, run_parallel,
    slope,
};

pub struct ScoreOptions {
    pub ticks: u64,
    pub every: u64,
    /// First tick of the scoring window.
    pub window_start: u64,
    /// A group with fewer cells than this counts as absent.
    pub n_min: u32,
}

impl ScoreOptions {
    pub fn from_args(args: &Args) -> Self {
        let ticks: u64 = args.parse("--ticks").unwrap_or(2000);
        let window_start = args.parse("--window").unwrap_or(ticks / 2);
        if window_start > ticks {
            die("--window must not be past --ticks");
        }
        ScoreOptions {
            ticks,
            every: crate::cli::every(args, 50),
            window_start,
            n_min: args.parse("--n-min").unwrap_or(10),
        }
    }
}

/// How one functional group fared in one run.
#[derive(Debug, Clone)]
pub struct GroupScore {
    pub min: f64,
    pub mean: f64,
    /// Coefficient of variation over the window.
    pub cv: f64,
    /// Slope of ln(n + 1) per 1000 ticks over the window: near 0 at
    /// equilibrium, negative in decline.
    pub log_slope: f64,
    /// Never below `n_min` in the window.
    pub persisted: bool,
    /// At some sample, at least `n_min` cells actually lived on this channel
    /// (`Census::diet_n`). Gene-classed founders do not count: at tick 0
    /// nobody has earned anything yet.
    pub established: bool,
    /// The tick it hit zero and stayed there, if it was ever present and did.
    pub extinct_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RunScore {
    pub seed: u64,
    pub final_pop: usize,
    pub mean_pop: f64,
    pub groups: Vec<GroupScore>,
    pub groups_persisting: u32,
    /// Earliest tick an established group went extinct.
    pub first_loss: Option<u64>,
    pub effective_groups: f64,
    pub rows_90: f64,
    pub non_default_phase: f64,
    pub ms_per_tick: f64,
    pub series: Vec<Census>,
}

impl RunScore {
    /// The scalar metrics compared between arms, by name.
    pub fn metrics(&self, opts: &ScoreOptions) -> Vec<(String, f64)> {
        let mut m = vec![
            ("groups_persisting".into(), self.groups_persisting as f64),
            ("effective_groups".into(), self.effective_groups),
            (
                "ticks_to_first_loss".into(),
                self.first_loss.unwrap_or(opts.ticks) as f64,
            ),
            ("rows_90".into(), self.rows_90),
            ("non_default_phase".into(), self.non_default_phase),
            ("mean_pop".into(), self.mean_pop),
        ];
        for (g, s) in self.groups.iter().enumerate() {
            m.push((format!("{}_mean", CLASSES[g]), s.mean));
        }
        m
    }

    fn to_json(&self, arm: &str) -> Value {
        let mut groups = serde_json::Map::new();
        for (g, s) in self.groups.iter().enumerate() {
            groups.insert(
                CLASSES[g].into(),
                json!({
                    "min": s.min, "mean": r3(s.mean), "cv": r3(s.cv),
                    "log_slope_per_1k": r3(s.log_slope), "persisted": s.persisted,
                    "established": s.established, "extinct_at": s.extinct_at,
                }),
            );
        }
        json!({
            "arm": arm, "seed": self.seed, "final_pop": self.final_pop,
            "mean_pop": r3(self.mean_pop), "groups_persisting": self.groups_persisting,
            "first_loss": self.first_loss, "effective_groups": r3(self.effective_groups),
            "rows_90": r3(self.rows_90), "non_default_phase": r3(self.non_default_phase),
            "ms_per_tick": r3(self.ms_per_tick), "groups": groups,
        })
    }
}

/// Run one seed and score it.
pub fn score_run(config: &WorldConfig, seed: u64, opts: &ScoreOptions) -> RunScore {
    let mut config = config.clone();
    config.seed = seed;
    let mut sim = Simulation::new(config);
    let mut series = vec![census::take(&sim)];
    let (mut ns, mut stepped) = (0u64, 0u64);
    for t in 1..=opts.ticks {
        if sim.world().population() > 0 {
            sim.step();
            ns += sim.world().stats.phase_ns.iter().sum::<u64>();
            stepped += 1;
        }
        if t % opts.every == 0 || t == opts.ticks {
            let mut c = census::take(&sim);
            c.tick = t; // an extinct world stops stepping, but time goes on
            series.push(c);
        }
    }
    score_series(seed, series, opts, ns as f64 / 1e6 / stepped.max(1) as f64)
}

pub fn score_series(seed: u64, series: Vec<Census>, opts: &ScoreOptions, ms: f64) -> RunScore {
    let window: Vec<&Census> = series
        .iter()
        .filter(|c| c.tick >= opts.window_start)
        .collect();
    let wn = window.len().max(1) as f64;
    let xs: Vec<f64> = window.iter().map(|c| c.tick as f64 / 1000.0).collect();

    let mut groups = Vec::new();
    for g in 0..GROUPS {
        let w: Vec<f64> = window.iter().map(|c| c.class_n[g] as f64).collect();
        let (mean, sd) = mean_sd(&w);
        let min = w.iter().cloned().fold(f64::INFINITY, f64::min);
        let min = if min.is_finite() { min } else { 0.0 };
        let logs: Vec<f64> = w.iter().map(|n| (n + 1.0).ln()).collect();
        let established = series.iter().any(|c| c.diet_n[g] >= opts.n_min);
        // Extinct: present at some sample, then zero at every sample after.
        let extinct_at = series
            .iter()
            .rposition(|c| c.class_n[g] > 0)
            .and_then(|last| series.get(last + 1))
            .map(|c| c.tick);
        groups.push(GroupScore {
            min,
            mean,
            cv: if mean > 0.0 { sd / mean } else { 0.0 },
            log_slope: slope(&xs, &logs),
            persisted: min >= opts.n_min as f64,
            established,
            extinct_at,
        });
    }
    let first_loss = groups
        .iter()
        .filter(|g| g.established)
        .filter_map(|g| g.extinct_at)
        .min();
    RunScore {
        seed,
        final_pop: series.last().map(|c| c.pop).unwrap_or(0),
        mean_pop: window.iter().map(|c| c.pop as f64).sum::<f64>() / wn,
        groups_persisting: groups.iter().filter(|g| g.persisted).count() as u32,
        first_loss,
        effective_groups: window.iter().map(|c| c.effective_groups()).sum::<f64>() / wn,
        rows_90: window.iter().map(|c| c.rows_90 as f64).sum::<f64>() / wn,
        non_default_phase: window.iter().map(|c| c.non_default_phase).sum::<f64>() / wn,
        ms_per_tick: ms,
        groups,
        series,
    }
}

pub fn run(args: &Args, base: WorldConfig) {
    let opts = ScoreOptions::from_args(args);
    let seeds = parse_seeds(args.value("--seeds").unwrap_or("1-10"));
    let jobs: usize = args.parse("--jobs").unwrap_or(3);

    let mut arms: Vec<(String, WorldConfig)> = vec![("A".into(), base.clone())];
    let vs_sets = args.values("--vs-set");
    if let Some(path) = args.value("--vs") {
        // B gets the same command-line overrides as A, then its own.
        let cfg = apply_overrides(args, read_config(path));
        arms.push(("B".into(), apply_sets(&cfg, &vs_sets)));
    } else if !vs_sets.is_empty() {
        arms.push(("B".into(), apply_sets(&base, &vs_sets)));
    }

    let jobs_list: Vec<(usize, u64)> = (0..arms.len())
        .flat_map(|a| seeds.iter().map(move |&s| (a, s)))
        .collect();
    let total = jobs_list.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let results = run_parallel(jobs_list, jobs, |&(a, seed)| {
        let r = score_run(&arms[a].1, seed, &opts);
        let d = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        eprintln!(
            "  [{d}/{total}] arm {} seed {seed}: pop {} persisting {} ({:.1} ms/tick)",
            arms[a].0, r.final_pop, r.groups_persisting, r.ms_per_tick
        );
        (a, r)
    });

    // Optional full time series, one JSON line per sample.
    if let Some(path) = args.value("--series") {
        let mut f = std::fs::File::create(path).unwrap_or_else(|e| die(&format!("{path}: {e}")));
        for (a, r) in &results {
            for c in &r.series {
                let line = json!({
                    "arm": arms[*a].0, "seed": r.seed, "tick": c.tick, "pop": c.pop,
                    "class_n": c.class_n, "gene_class_n": c.gene_class_n,
                    "rows_90": c.rows_90, "non_default_phase": r3(c.non_default_phase),
                });
                writeln!(f, "{line}").unwrap();
            }
        }
    }

    for (a, r) in &results {
        println!("{}", r.to_json(&arms[*a].0));
    }

    let per_arm: Vec<Vec<&RunScore>> = (0..arms.len())
        .map(|a| {
            results
                .iter()
                .filter(|(x, _)| *x == a)
                .map(|(_, r)| r)
                .collect()
        })
        .collect();
    let mut summary = serde_json::Map::new();
    for (a, runs) in per_arm.iter().enumerate() {
        summary.insert(arms[a].0.clone(), summarize(runs, &opts));
    }
    if arms.len() == 2 {
        summary.insert(
            "paired_B_minus_A".into(),
            paired(&per_arm[0], &per_arm[1], &opts),
        );
    }
    let summary = Value::Object(summary);
    println!("{}", json!({ "summary": summary }));
    eprint!("{}", table(&arms, &per_arm, &opts));
}

fn summarize(runs: &[&RunScore], opts: &ScoreOptions) -> Value {
    let n = runs.len() as f64;
    let mut out = serde_json::Map::new();
    out.insert("seeds".into(), json!(runs.len()));
    for (name, _) in runs[0].metrics(opts) {
        let v: Vec<f64> = runs
            .iter()
            .map(|r| {
                r.metrics(opts)
                    .into_iter()
                    .find(|(k, _)| *k == name)
                    .unwrap()
                    .1
            })
            .collect();
        let (m, sd) = mean_sd(&v);
        out.insert(name, json!({ "mean": r3(m), "sd": r3(sd) }));
    }
    let mut persisted = serde_json::Map::new();
    for (g, name) in CLASSES.iter().enumerate().take(GROUPS) {
        let k = runs.iter().filter(|r| r.groups[g].persisted).count();
        persisted.insert(name.to_string(), json!(r3(k as f64 / n)));
    }
    out.insert("share_of_seeds_persisting".into(), Value::Object(persisted));
    let lost: Vec<u64> = runs.iter().filter_map(|r| r.first_loss).collect();
    out.insert(
        "share_of_seeds_losing_a_group".into(),
        json!(r3(lost.len() as f64 / n)),
    );
    Value::Object(out)
}

fn paired(a: &[&RunScore], b: &[&RunScore], opts: &ScoreOptions) -> Value {
    let mut out = serde_json::Map::new();
    for (name, _) in a[0].metrics(opts) {
        let diffs: Vec<f64> = a
            .iter()
            .zip(b)
            .map(|(ra, rb)| {
                let get = |r: &RunScore| {
                    r.metrics(opts)
                        .into_iter()
                        .find(|(k, _)| *k == name)
                        .unwrap()
                        .1
                };
                get(rb) - get(ra)
            })
            .collect();
        let (m, sd) = mean_sd(&diffs);
        let se = sd / (diffs.len() as f64).sqrt();
        out.insert(name, json!({ "mean_diff": r3(m), "se": r3(se) }));
    }
    Value::Object(out)
}

/// Human-readable summary for the terminal.
fn table(
    arms: &[(String, WorldConfig)],
    per_arm: &[Vec<&RunScore>],
    opts: &ScoreOptions,
) -> String {
    let mut s = format!(
        "\nscore: {} ticks, window from t={}, n_min {}, {} seeds\n",
        opts.ticks,
        opts.window_start,
        opts.n_min,
        per_arm[0].len()
    );
    s.push_str(
        "arm  seed   pop  photo thermo  scav hunter | persist eff  rows90 phase first_loss\n",
    );
    for (a, runs) in per_arm.iter().enumerate() {
        for r in runs {
            let g = |i: usize| r.groups[i].mean;
            s.push_str(&format!(
                "{:<4} {:>4} {:>5} {:>6.0} {:>6.0} {:>5.0} {:>6.0} | {:>7} {:>4.2} {:>6.1} {:>5.2} {}\n",
                arms[a].0,
                r.seed,
                r.final_pop,
                g(0),
                g(1),
                g(2),
                g(3),
                r.groups_persisting,
                r.effective_groups,
                r.rows_90,
                r.non_default_phase,
                r.first_loss.map(|t| t.to_string()).unwrap_or_else(|| "-".into()),
            ));
        }
    }
    for (a, runs) in per_arm.iter().enumerate() {
        let n = runs.len() as f64;
        let share = |g: usize| runs.iter().filter(|r| r.groups[g].persisted).count() as f64 / n;
        let (pm, psd) = mean_sd(
            &runs
                .iter()
                .map(|r| r.groups_persisting as f64)
                .collect::<Vec<_>>(),
        );
        s.push_str(&format!(
            "arm {}: groups persisting {:.2} ± {:.2}; share of seeds keeping photo {:.2} thermo {:.2} scav {:.2} hunter {:.2}\n",
            arms[a].0, pm, psd, share(0), share(1), share(2), share(3)
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample where every counted cell has actually lived on its channel.
    fn census(tick: u64, groups: [u32; 4]) -> Census {
        let n = [groups[0], groups[1], groups[2], groups[3], 0];
        Census {
            tick,
            pop: groups.iter().sum::<u32>() as usize,
            class_n: n,
            diet_n: n,
            ..Census::default()
        }
    }

    fn opts() -> ScoreOptions {
        ScoreOptions {
            ticks: 400,
            every: 100,
            window_start: 200,
            n_min: 10,
        }
    }

    #[test]
    fn a_lost_group_is_dated_and_a_steady_one_persists() {
        let series = vec![
            census(0, [100, 50, 50, 50]),
            census(100, [100, 20, 50, 30]),
            census(200, [100, 20, 50, 5]),
            census(300, [100, 20, 50, 0]),
            census(400, [100, 20, 50, 0]),
        ];
        let s = score_series(1, series, &opts(), 1.0);
        assert!(s.groups[0].persisted && s.groups[1].persisted && s.groups[2].persisted);
        assert!(!s.groups[3].persisted);
        assert_eq!(s.groups[3].extinct_at, Some(300));
        assert_eq!(s.first_loss, Some(300));
        assert_eq!(s.groups_persisting, 3);
        assert_eq!(s.groups[0].extinct_at, None);
    }

    /// At tick 0 every cell is classed by its genes. A group that only ever
    /// existed that way never lived on its channel, so its disappearance is
    /// not a loss; a group that did live on it and then crashed early is.
    #[test]
    fn a_group_is_established_by_what_it_lived_on_not_its_genes() {
        let mut gene_only = census(0, [100, 50, 50, 60]);
        gene_only.diet_n = [0, 0, 0, 0, 260];
        let crashed = || {
            vec![
                census(0, [100, 50, 50, 0]),
                census(100, [160, 50, 50, 0]),
                census(200, [160, 50, 50, 0]),
                census(300, [160, 50, 50, 0]),
                census(400, [160, 50, 50, 0]),
            ]
        };
        let mut series = crashed();
        series[0] = gene_only;
        let s = score_series(1, series, &opts(), 1.0);
        assert!(!s.groups[3].established);
        assert_eq!(s.first_loss, None);

        // Predators that fed (diet) and then died out by t=100: a loss.
        let mut series = crashed();
        series[0].class_n[3] = 60;
        series[0].diet_n[3] = 60;
        let s = score_series(1, series, &opts(), 1.0);
        assert!(s.groups[3].established);
        assert_eq!(s.first_loss, Some(100));
    }

    /// A group never present has no extinction date.
    #[test]
    fn a_group_that_never_existed_has_no_extinction_date() {
        let series = vec![census(0, [10, 0, 0, 0]), census(100, [10, 0, 0, 0])];
        let s = score_series(1, series, &opts(), 1.0);
        assert_eq!(s.groups[1].extinct_at, None);
    }

    #[test]
    fn effective_groups_counts_how_evenly_the_groups_share_the_world() {
        // A monoculture has one effective group; four equal groups have four.
        assert!((census(0, [10, 0, 0, 0]).effective_groups() - 1.0).abs() < 1e-9);
        assert!((census(0, [10, 10, 10, 10]).effective_groups() - 4.0).abs() < 1e-9);
    }
}
