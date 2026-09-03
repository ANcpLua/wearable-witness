#![allow(clippy::format_push_string)] // table rendering; the allocation is not the point
//! `ww` — the command surface over ww-core.
//!
//! Exit codes are the point of `refute`: it is meant to run in CI over a
//! published corpus, so a refutation has to be machine-visible without
//! parsing prose. `report` refuses to run over a corpus that fails `verify`.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use ww_core::{
    claim::Claim,
    corpus::Corpus,
    report::{self, Policy, Pooled, Row, Verdict},
    series::Series,
    stats::{Standing, Z_95, Z_99},
    Alignment,
};

#[derive(Parser)]
#[command(name = "ww", version, about = "Hold a wearable's accuracy against an attested reference")]
struct Cli {
    /// Corpus root; holds index.json and the raw files it names.
    #[arg(long, default_value = "corpus", global = true)]
    corpus: PathBuf,
    /// Directory of derived series (reference beats, device rates, controls).
    #[arg(long, default_value = "series", global = true)]
    series: PathBuf,
    /// Claims ledger.
    #[arg(long, default_value = "claims/claims.json", global = true)]
    claims: PathBuf,
    /// Use a 99% interval instead of 95%.
    #[arg(long, global = true)]
    strict: bool,
    /// Treat every recording as if its clocks were attested shared: alignment
    /// is reported but never gates. A counterfactual, and labelled as one.
    #[arg(long, global = true)]
    assume_shared_clock: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Re-hash every corpus file and check every series against it.
    Verify,
    /// Print the tables. Refuses to run over a corpus that fails Verify.
    Report {
        /// Emit everything as JSON instead of Markdown.
        #[arg(long)]
        json: bool,
    },
    /// Exit 1 if any testable claim is refuted on any row.
    Refute,
}

fn load_corpus(root: &Path) -> Result<Corpus> {
    let p = root.join("index.json");
    let s = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
    serde_json::from_str(&s).with_context(|| format!("parsing {}", p.display()))
}

fn load_series(dir: &Path, corpus: &Corpus) -> Result<Vec<Series>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort(); // deterministic order regardless of filesystem enumeration
    let mut out = Vec::new();
    for p in &paths {
        let s = std::fs::read_to_string(p)?;
        let series: Series = serde_json::from_str(&s).with_context(|| format!("parsing {}", p.display()))?;
        if let Some(flaw) = series.check(corpus) {
            bail!("{}: {flaw}", p.display());
        }
        out.push(series);
    }
    Ok(out)
}

fn load_claims(p: &Path) -> Result<Vec<Claim>> {
    if !p.exists() {
        return Ok(Vec::new());
    }
    let s = std::fs::read_to_string(p)?;
    serde_json::from_str(&s).with_context(|| format!("parsing {}", p.display()))
}

fn verified_corpus(root: &Path) -> Result<Corpus> {
    let corpus = load_corpus(root)?;
    let defects = corpus.verify(root);
    if !defects.is_empty() {
        for d in &defects {
            eprintln!("  {d}");
        }
        bail!("{} corpus defect(s); a report over this corpus would not be reproducible", defects.len());
    }
    Ok(corpus)
}

fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

fn interval_pct(i: (f64, f64)) -> String {
    format!("{:.1}–{:.1}%", i.0 * 100.0, i.1 * 100.0)
}

fn lag_cell(a: Option<&Alignment>, applied: Option<f64>) -> String {
    match a {
        Some(Alignment::Conclusive { lag_s, corr, .. }) => {
            let applied = applied.map_or(String::new(), |l| if (l - lag_s).abs() > 1e-9 { format!(", applied {l:+.0} s") } else { String::new() });
            format!("{lag_s:+.0} s (r={corr:.2}{applied})")
        }
        Some(Alignment::Ambiguous { best_lag_s, corr, runner_up, .. }) => {
            format!("ambiguous (best {best_lag_s:+.0} s, r={corr:.2}, rival {runner_up:.2})")
        }
        None => "—".into(),
    }
}

fn standing_cell(v: &[report::ClaimVerdict]) -> String {
    if v.is_empty() {
        return "no testable claim".into();
    }
    v.iter()
        .map(|c| {
            let s = match c.standing {
                Standing::Refuted => "**FAILS**",
                Standing::Consistent => "not settled",
                Standing::Better => "meets",
            };
            format!("{}: {s}", c.claim)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn hr_cells(v: &Verdict) -> [String; 5] {
    match v {
        Verdict::Evaluated { hr, verdicts, .. } => [
            format!("{:+.1} ± {:.1}", hr.bland_altman.bias, hr.bland_altman.sd),
            format!("{:+.1} to {:+.1}", hr.bland_altman.loa.0, hr.bland_altman.loa.1),
            format!("{} [{}]", pct(hr.mape.mape), interval_pct(hr.mape.interval)),
            format!("{}/{} = {} [{}]", hr.tolerance.within, hr.tolerance.n, pct(hr.tolerance.fraction), interval_pct(hr.tolerance.interval)),
            standing_cell(verdicts),
        ],
        Verdict::NotEvaluated { why } => ["—".into(), "—".into(), "—".into(), "—".into(), format!("not evaluated: {why}")],
    }
}

fn rows_table(rows: &[&Row], level: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "| Recording | Activity | Device | Lag | Epochs | Bias ± SD (bpm) | LoA (bpm) | MAPE [{level}] | Within ±5 bpm / ±10% [{level}] | Verdict |\n\
         |---|---|---|---|---:|---:|---|---|---|---|\n"
    ));
    for r in rows {
        let [bias, loa, mape, tol, verdict] = hr_cells(&r.verdict);
        out.push_str(&format!(
            "| {} | {} | {} | {} | {}/{} | {bias} | {loa} | {mape} | {tol} | {verdict} |\n",
            r.recording,
            r.activity,
            r.device,
            lag_cell(r.alignment.as_ref(), r.lag_applied_s),
            r.coverage.evaluable,
            r.coverage.windows
        ));
    }
    out
}

fn pooled_table(pooled: &[Pooled], level: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "| Device | Activity | Recordings | Epochs | Bias ± SD (bpm) | LoA (bpm) | MAPE [{level}] | Within ±5 bpm / ±10% [{level}] | Verdict |\n\
         |---|---|---:|---:|---:|---|---|---|---|\n"
    ));
    for p in pooled {
        let [bias, loa, mape, tol, verdict] = hr_cells(&p.verdict);
        let n = match &p.verdict {
            Verdict::Evaluated { hr, .. } => hr.n.to_string(),
            Verdict::NotEvaluated { .. } => "—".into(),
        };
        out.push_str(&format!(
            "| {} | {} | {} | {n} | {bias} | {loa} | {mape} | {tol} | {verdict} |\n",
            p.device,
            p.activity.as_deref().unwrap_or("**all**"),
            p.recordings
        ));
    }
    if pooled.is_empty() {
        out.push_str("| _(nothing evaluated)_ | | | | | | | | |\n");
    }
    out
}

fn hrv_table(rows: &[&Row], level: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "| Recording | Device | Epochs (60 s) | RMSSD bias ± SD (ms) | LoA (ms) | MAPE [{level}] |\n|---|---|---:|---:|---|---|\n"
    ));
    let mut any = false;
    for r in rows {
        if let Verdict::Evaluated { hrv: Some(h), .. } = &r.verdict {
            any = true;
            out.push_str(&format!(
                "| {} | {} | {} | {:+.1} ± {:.1} | {:+.1} to {:+.1} | {} [{}] |\n",
                r.recording,
                r.device,
                h.n,
                h.bland_altman.bias,
                h.bland_altman.sd,
                h.bland_altman.loa.0,
                h.bland_altman.loa.1,
                pct(h.mape.mape),
                interval_pct(h.mape.interval)
            ));
        }
    }
    if !any {
        out.push_str("| _(no device series carries beats)_ | | | | | |\n");
    }
    out
}

fn ledger_table(claims: &[Claim]) -> String {
    let mut out = String::from("| Claim | Applies to | Testable | Quote |\n|---|---|---|---|\n");
    for c in claims {
        let applies = match &c.applies_to {
            ww_core::claim::AppliesTo::Any => "any device".to_string(),
            ww_core::claim::AppliesTo::Device { name } => name.clone(),
        };
        let testable = if c.is_testable() { "yes" } else { "no: pins no bound" };
        let quote: String = c.quote.chars().take(160).collect();
        let ellipsis = if c.quote.chars().count() > 160 { "…" } else { "" };
        out.push_str(&format!("| `{}` | {applies} | {testable} | “{quote}{ellipsis}” |\n", c.id));
    }
    out
}

fn markdown(rows: &[Row], pooled: &[Pooled], claims: &[Claim], p: &Policy) -> String {
    let controls: Vec<&Row> = rows.iter().filter(|r| r.is_control()).collect();
    let measured: Vec<&Row> = rows.iter().filter(|r| !r.is_control()).collect();
    let mut out = String::new();
    if p.assume_shared_clock {
        out.push_str("**Counterfactual: every clock treated as shared.** Alignment is shown but does not gate; a lag of 0 s is applied to every row regardless of the estimate. Nothing below attests synchronisation.\n\n");
    }
    out.push_str(&format!(
        "Policy: {} intervals; epochs {:.0} s (HRV {:.0} s); at least {} evaluable epochs per row; alignment searched ±{:.0} s, conclusive at r ≥ {:.2} with margin ≥ {:.2} over rivals beyond ±{:.0} s; tolerance rule ±{:.0} bpm or ±{:.0}%, whichever is greater.\n\n",
        p.level, p.window.hr_window_s, p.window.hrv_window_s, p.min_windows, p.alignment.max_lag_s, p.alignment.min_corr, p.alignment.min_margin, p.alignment.exclusion_s, p.tolerance.bpm, p.tolerance.percent
    ));
    out.push_str("## Instrument controls\n\n");
    out.push_str(&rows_table(&controls, &p.level));
    out.push_str("\n## Measured\n\n");
    out.push_str(&rows_table(&measured, &p.level));
    out.push_str("\n## Pooled\n\n");
    out.push_str(&pooled_table(pooled, &p.level));
    out.push_str("\n## Heart-rate variability (descriptive)\n\n");
    out.push_str(&hrv_table(&measured, &p.level));
    out.push_str("\n## Claims ledger\n\n");
    out.push_str(&ledger_table(claims));
    out
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut policy = if cli.strict { Policy::new(Z_99, "99%") } else { Policy::new(Z_95, "95%") };
    policy.assume_shared_clock = cli.assume_shared_clock;

    match cli.cmd {
        Cmd::Verify => {
            let corpus = load_corpus(&cli.corpus)?;
            let defects = corpus.verify(&cli.corpus);
            if !defects.is_empty() {
                for d in &defects {
                    println!("  {d}");
                }
                bail!("{} defect(s)", defects.len());
            }
            let series = load_series(&cli.series, &corpus)?;
            let files = corpus.files().len();
            let devices = series.iter().filter(|s| s.role == ww_core::series::Role::Device).count();
            println!(
                "{} recordings, {files} raw files, {} series ({devices} device runs), no defects",
                corpus.recordings.len(),
                series.len()
            );
            Ok(())
        }
        Cmd::Report { json } => {
            let corpus = verified_corpus(&cli.corpus)?;
            let series = load_series(&cli.series, &corpus)?;
            let claims = load_claims(&cli.claims)?;
            let rows = report::build(&corpus, &series, &claims, &policy);
            let pooled = report::pool(&rows, &claims, &policy);
            if json {
                let v = serde_json::json!({
                    "policy": policy,
                    "sources": corpus.sources,
                    "claims": claims,
                    "rows": rows,
                    "pooled": pooled,
                });
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", markdown(&rows, &pooled, &claims, &policy));
            }
            Ok(())
        }
        Cmd::Refute => {
            let corpus = verified_corpus(&cli.corpus)?;
            let series = load_series(&cli.series, &corpus)?;
            let claims = load_claims(&cli.claims)?;
            let rows = report::build(&corpus, &series, &claims, &policy);
            let mut refuted = 0;
            for r in rows.iter().filter(|r| !r.is_control()) {
                if let Verdict::Evaluated { hr, verdicts, .. } = &r.verdict {
                    for v in verdicts.iter().filter(|v| v.standing == Standing::Refuted) {
                        refuted += 1;
                        println!(
                            "{} / {}: fails `{}` — MAPE {} [{}] over {} epochs",
                            r.recording,
                            r.device,
                            v.claim,
                            pct(hr.mape.mape),
                            interval_pct(hr.mape.interval),
                            hr.n
                        );
                    }
                }
            }
            if refuted == 0 {
                println!("no claim refuted at {}", policy.level);
                return Ok(());
            }
            std::process::exit(1);
        }
    }
}
