//! Argument parsing, config loading and the small thread pool the lab shares
//! between its modes.

use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use primordium::config::{SeedStrategy, WorldConfig};

/// The raw command line, queried by flag name.
pub struct Args(Vec<String>);

/// Flags that take a value.
const VALUE_FLAGS: &[&str] = &[
    "--config",
    "--seed",
    "--w",
    "--h",
    "--cells",
    "--clusters",
    "--set",
    "--ticks",
    "--every",
    "--render",
    "--shots",
    "--seeds",
    "--jobs",
    "--window",
    "--n-min",
    "--series",
    "--vs",
    "--vs-set",
    "--invade",
    "--at",
    "--n",
    "--rows",
    "--rare-cap",
];
/// Flags that stand alone.
const BOOL_FLAGS: &[&str] = &[
    "--uniform",
    "--phase-detail",
    "--traits",
    "--archetypes",
    "--color-check",
    "--niche",
    "--dump-config",
    "--score",
    "--hash",
];

impl Args {
    /// The command line, validated: an unknown flag, a stray positional
    /// argument or a flag missing its value is an error. Silently ignoring
    /// `--tick 3` would run the default 2000 ticks and report it as measured.
    pub fn from_env() -> Self {
        let args = Args(std::env::args().skip(1).collect());
        if let Err(e) = args.validate() {
            die(&e);
        }
        args
    }

    fn validate(&self) -> Result<(), String> {
        let mut i = 0;
        while i < self.0.len() {
            let a = self.0[i].as_str();
            if VALUE_FLAGS.contains(&a) {
                if self.0.get(i + 1).is_none() {
                    return Err(format!("{a} needs a value"));
                }
                i += 2;
            } else if BOOL_FLAGS.contains(&a) {
                i += 1;
            } else if a.starts_with("--") {
                return Err(format!("unknown flag {a}"));
            } else {
                return Err(format!(
                    "unexpected argument {a:?} (a config file goes after --config)"
                ));
            }
        }
        Ok(())
    }

    pub fn flag(&self, name: &str) -> bool {
        self.0.iter().any(|a| a == name)
    }

    /// The value after the first occurrence of `name`.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values(name).into_iter().next()
    }

    /// The values after every occurrence of `name`, for repeatable flags.
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.0
            .windows(2)
            .filter(|w| w[0] == name)
            .map(|w| w[1].as_str())
            .collect()
    }

    /// `value(name)` parsed; exits with a message on a malformed value rather
    /// than silently falling back to a default.
    pub fn parse<T: FromStr>(&self, name: &str) -> Option<T> {
        self.value(name).map(|v| {
            v.parse()
                .unwrap_or_else(|_| die(&format!("{name}: cannot parse {v:?}")))
        })
    }
}

pub fn die(msg: &str) -> ! {
    eprintln!("lab: {msg}");
    std::process::exit(2)
}

/// Build the config: `--config file.json` (missing fields take their
/// defaults) or `WorldConfig::default()`, then the shorthand flags, then
/// every `--set key=value` in order.
pub fn load_config(args: &Args) -> WorldConfig {
    let config = match args.value("--config") {
        Some(path) => read_config(path),
        None => WorldConfig::default(),
    };
    apply_overrides(args, config)
}

/// A config file; fields it leaves out take their defaults, unknown fields are
/// an error.
pub fn read_config(path: &str) -> WorldConfig {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| die(&format!("reading {path}: {e}")));
    serde_json::from_str(&text).unwrap_or_else(|e| die(&format!("parsing {path}: {e}")))
}

/// The command-line overrides on top of a config: the shorthand flags, then
/// every `--set` in order. Both arms of a paired comparison get these, so the
/// only difference between them is the one being measured.
pub fn apply_overrides(args: &Args, mut config: WorldConfig) -> WorldConfig {
    if let Some(v) = args.parse("--seed") {
        config.seed = v;
    }
    if let Some(v) = args.parse("--w") {
        config.grid_width = v;
    }
    if let Some(v) = args.parse("--h") {
        config.grid_height = v;
    }
    if let Some(v) = args.parse("--cells") {
        config.initial_cell_count = v;
    }
    if let Some(v) = args.parse("--clusters") {
        config.cluster_count = v;
    }
    if args.flag("--uniform") {
        config.initial_genome_strategy = SeedStrategy::RandomUniform;
    }
    apply_sets(&config, &args.values("--set"))
}

/// Apply `key=value` overrides. The value is read as JSON (`3`, `true`,
/// `[0.5,0.5,0,0]`), or as a string when it is not JSON (`random_uniform`).
/// An unknown key is an error: a typo must not silently measure the default.
pub fn apply_sets(config: &WorldConfig, sets: &[&str]) -> WorldConfig {
    let mut value = serde_json::to_value(config).expect("config serializes");
    for set in sets {
        let (key, raw) = set
            .split_once('=')
            .unwrap_or_else(|| die(&format!("--set {set}: expected key=value")));
        let obj = value.as_object_mut().expect("config is an object");
        if !obj.contains_key(key) {
            die(&format!("--set: WorldConfig has no field {key:?}"));
        }
        let parsed = serde_json::from_str(raw).unwrap_or(serde_json::Value::String(raw.into()));
        obj.insert(key.to_string(), parsed);
    }
    serde_json::from_value(value).unwrap_or_else(|e| die(&format!("--set: {e}")))
}

/// `"1-10"`, `"3"` or `"1,4,9"` (ranges allowed inside a list).
pub fn parse_seeds(spec: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let num = |s: &str| -> u64 {
            s.trim()
                .parse()
                .unwrap_or_else(|_| die(&format!("--seeds: bad seed {s:?}")))
        };
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (num(a), num(b));
                if a > b {
                    die(&format!("--seeds: range {a}-{b} is backwards"));
                }
                out.extend(a..=b)
            }
            None => out.push(num(part)),
        }
    }
    if out.is_empty() {
        die("--seeds: no seeds");
    }
    out
}

/// A report interval of at least one tick.
pub fn every(args: &Args, default: u64) -> u64 {
    let every = args.parse("--every").unwrap_or(default);
    if every == 0 {
        die("--every must be at least 1");
    }
    every
}

/// Run `f` over `items` on at most `jobs` threads, returning results in
/// input order. Each simulation is single-threaded and owns its RNG, so
/// running several side by side changes nothing about any of them.
pub fn run_parallel<T, R, F>(items: Vec<T>, jobs: usize, f: F) -> Vec<R>
where
    T: Send + Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1).min(items.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}

/// Least-squares slope of `ys` against `xs`.
pub fn slope(xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    if sxx == 0.0 { 0.0 } else { sxy / sxx }
}

/// `(mean, sample standard deviation)`.
pub fn mean_sd(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let m = v.iter().sum::<f64>() / n;
    let sd = if n > 1.0 {
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    (m, sd)
}

/// Round for JSON output, so reports stay readable.
pub fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_accept_ranges_and_lists() {
        assert_eq!(parse_seeds("1-3"), vec![1, 2, 3]);
        assert_eq!(parse_seeds("5"), vec![5]);
        assert_eq!(parse_seeds("1,4-5,9"), vec![1, 4, 5, 9]);
    }

    #[test]
    fn set_overrides_numbers_bools_arrays_and_enums() {
        let c = apply_sets(
            &WorldConfig::default(),
            &[
                "max_move_distance=3",
                "attack_only_when_harmful=true",
                "archetype_population_shares=[0.5,0.5,0,0]",
                "initial_genome_strategy=preset_archetypes",
            ],
        );
        assert_eq!(c.max_move_distance, 3);
        assert!(c.attack_only_when_harmful);
        assert_eq!(c.archetype_population_shares, [0.5, 0.5, 0.0, 0.0]);
        assert!(matches!(
            c.initial_genome_strategy,
            SeedStrategy::PresetArchetypes
        ));
    }

    #[test]
    fn unknown_flags_and_stray_arguments_are_rejected() {
        let args = |v: &[&str]| Args(v.iter().map(|s| s.to_string()).collect());
        assert!(
            args(&["--score", "--seeds", "1-3", "--set", "seed=4"])
                .validate()
                .is_ok()
        );
        assert!(args(&["--tick", "3"]).validate().is_err());
        assert!(args(&["default.json"]).validate().is_err());
        assert!(args(&["--seeds"]).validate().is_err());
    }

    #[test]
    fn slope_of_a_line() {
        assert!((slope(&[0.0, 1.0, 2.0], &[1.0, 3.0, 5.0]) - 2.0).abs() < 1e-12);
    }
}
