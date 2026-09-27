//! The lab: Primordium's headless measurement harness.
//!
//! Modes (default: run):
//!
//! ```text
//! lab [config]                        run one sim, one JSON report line per --every ticks
//! lab --score  [config] [arms]        benchmark: many seeds, windowed scorecard
//! lab --invade <strategy> [config]    growth-from-rare assay
//! lab --archetypes | --niche | --color-check | --render <prefix> | --dump-config
//! lab --hash [config]                 full-state hash per seed every --every ticks
//! ```
//!
//! Config: `--config file.json` (fields it leaves out take their defaults,
//! otherwise `WorldConfig::default()`), then `--seed --w --h --cells
//! --clusters --uniform`, then every `--set key=value` in order.
//!
//! Run: `--ticks N` (2000) `--every N` (100) `--phase-detail` `--traits`.
//! Score: `--seeds 1-10` `--jobs 3` `--ticks 2000` `--every 50`
//! `--window T` (ticks/2) `--n-min 10` `--series out.jsonl`, and a second arm
//! with `--vs-set key=value` (repeatable) or `--vs file.json`; both arms get
//! the command line's other overrides.
//! Invade: `--at 1000` `--n 30` `--ticks at+800` `--rows top,depth`
//! `--rare-cap 5n` `--seeds 1-5` `--jobs 3`.
//! Hash: `--seeds 1-2` `--ticks 600` `--every 100` `--jobs 3`.
//!
//! Unknown flags and stray arguments are errors.
//!
//! Output is JSON lines on stdout; progress and summaries go to stderr.

// Same reason as in lib.rs.
#![allow(clippy::empty_line_after_doc_comments)]

mod census;
mod cli;
mod invade;
mod report;
mod score;
mod tools;

use primordium::config::WorldConfig;

use crate::cli::{Args, load_config};

// @veridikt
// kind: module
// name: Lab
// purpose: "Headless measurement harness: per-interval reports, the multi-seed scorecard, the invasion assay and one-shot economy/colour/niche tools"
// owner: "primordium-maintainers"
// because: "Balance work is only as good as its measurements; the harness builds from the same tree as the sim and reads the sim's own counters, so it cannot drift the way the old out-of-tree patch did"
// depends_on: Sim, Render
fn main() {
    let args = Args::from_env();
    if args.flag("--dump-config") {
        println!(
            "{}",
            serde_json::to_string_pretty(&WorldConfig::default()).expect("config serializes")
        );
        return;
    }
    let config = load_config(&args);

    if let Some(prefix) = args.value("--render") {
        let ticks: u64 = args.parse("--ticks").unwrap_or(500);
        let shots: Vec<u64> = args
            .value("--shots")
            .map(str::to_string)
            .unwrap_or_else(|| format!("0,10,{ticks}"))
            .split(',')
            .filter_map(|v| v.parse().ok())
            .collect();
        tools::render_frames(&config, ticks, &shots, prefix);
    } else if args.flag("--niche") {
        tools::niche(&config);
    } else if args.flag("--color-check") {
        tools::color_check(&config);
    } else if args.flag("--archetypes") {
        tools::archetypes(&config);
    } else if args.flag("--hash") {
        let seeds = cli::parse_seeds(args.value("--seeds").unwrap_or("1-2"));
        let ticks: u64 = args.parse("--ticks").unwrap_or(600);
        let every = cli::every(&args, 100);
        tools::hash(
            &config,
            seeds,
            ticks,
            every,
            args.parse("--jobs").unwrap_or(3),
        );
    } else if args.flag("--score") {
        score::run(&args, config);
    } else if args.value("--invade").is_some() {
        invade::run(&args, config);
    } else {
        report::run(&args, config);
    }
}
