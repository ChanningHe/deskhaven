//! Markdown rendering of a [`Report`].

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use super::{Report, mib};
use crate::dbstats::{RowWrites, Snapshot};
use crate::stats::{Phase, Summary};

impl Display for Report {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "# {}\n", self.title)?;
        self.header(f)?;
        self.nfr_table(f)?;
        self.parameters(f)?;
        self.requests(f)?;
        self.process(f)?;
        self.database(f)
    }
}

impl Report {
    fn header(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let m = &self.machine;
        let build = match cfg!(debug_assertions) {
            true => "debug",
            false => "release",
        };
        let exit = match self.server_exit_clean {
            true => "clean (status 0 after SIGTERM)",
            false => "NOT clean",
        };
        writeln!(f, "| | |\n|---|---|")?;
        writeln!(
            f,
            "| Machine | {}, {} logical cores |",
            m.cpu_model, m.logical_cores
        )?;
        writeln!(f, "| Memory / kernel | {} / Linux {} |", m.memory, m.kernel)?;
        writeln!(f, "| Load generator build | {build} |")?;
        writeln!(f, "| Server shutdown | {exit} |")?;
        writeln!(f, "| Command | `{}` |\n", self.command)
    }

    fn nfr_table(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "## NFR verdicts (01-prd §4, steady window)\n")?;
        writeln!(f, "| NFR | Target | Measured | Result |\n|---|---|---|---|")?;
        for v in self.verdicts() {
            let result = if v.pass { "PASS" } else { "**FAIL**" };
            writeln!(
                f,
                "| {} | {} | {} | {result} |",
                v.nfr, v.target, v.measured
            )?;
        }
        writeln!(f)
    }

    fn parameters(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "## Parameters\n\n| Parameter | Value |\n|---|---|")?;
        for (name, value) in &self.parameters {
            writeln!(f, "| {name} | {value} |")?;
        }
        writeln!(f)
    }

    fn requests(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "## Requests (client-observed, connect included)\n")?;
        writeln!(
            f,
            "| Phase | Request | OK | Failed | p50 | p90 | p99 | max |\n|---|---|---|---|---|---|---|---|"
        )?;
        for (phase, kind, s) in &self.summaries {
            writeln!(
                f,
                "| {} | {kind} | {} | {} | {} | {} | {} | {} |",
                phase_name(*phase),
                s.ok,
                s.failed(),
                latency(s.p50),
                latency(s.p90),
                latency(s.p99),
                latency(s.max)
            )?;
        }
        writeln!(f)?;
        failures(f, &self.summaries)
    }

    fn process(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let cores = |c: Option<f64>| c.map_or_else(|| "n/a".to_owned(), |c| format!("{c:.2}"));
        let bytes = |b: Option<u64>| b.map_or_else(|| "n/a".to_owned(), mib);
        writeln!(f, "## Server process (`/proc/<pid>` of deskhaven only)\n")?;
        writeln!(f, "| Metric | Value |\n|---|---|")?;
        writeln!(
            f,
            "| RSS, idle (max) | {} |",
            bytes(self.measured.idle_rss_bytes)
        )?;
        writeln!(
            f,
            "| RSS, steady (max) | {} |",
            bytes(self.steady_rss_max_bytes)
        )?;
        writeln!(
            f,
            "| CPU cores, steady average | {} |",
            cores(self.measured.cpu_cores)
        )?;
        writeln!(
            f,
            "| CPU cores, busiest 10 s of steady | {} |",
            cores(self.cpu_peak_cores)
        )?;
        writeln!(
            f,
            "| Bytes sent to storage, steady (`write_bytes`; includes `server.log`) | {} |\n",
            bytes(self.steady_write_bytes)
        )
    }

    fn database(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "## Database row writes (inserted / updated / deleted)\n")?;
        writeln!(f, "| Table | Steady window | Whole run |\n|---|---|---|")?;
        let names: BTreeSet<&String> = self
            .db_run
            .tables
            .keys()
            .chain(self.db_steady.tables.keys())
            .collect();
        for name in names {
            let steady = rows(self.db_steady.table(name));
            writeln!(
                f,
                "| {name} | {steady} | {} |",
                rows(self.db_run.table(name))
            )?;
        }
        commits(f, &self.db_steady, &self.db_run)
    }
}

fn failures(
    f: &mut Formatter<'_>,
    summaries: &[(Phase, crate::stats::Kind, Summary)],
) -> fmt::Result {
    let mut any = false;
    for (phase, kind, s) in summaries {
        for (failure, count) in &s.failures {
            if !any {
                writeln!(f, "Failures:\n")?;
                any = true;
            }
            writeln!(f, "- {} / {kind}: {failure} × {count}", phase_name(*phase))?;
        }
    }
    if any { writeln!(f) } else { Ok(()) }
}

fn commits(f: &mut Formatter<'_>, steady: &Snapshot, run: &Snapshot) -> fmt::Result {
    match (steady.commits, run.commits) {
        (Some(steady), Some(run)) => writeln!(
            f,
            "\nCommitted transactions (`pg_stat_database.xact_commit`, reads included): \
             {steady} in the steady window, {run} in the whole run."
        ),
        _ => Ok(()),
    }
}

fn rows(w: RowWrites) -> String {
    format!("{} / {} / {}", w.inserted, w.updated, w.deleted)
}

fn latency(d: Option<Duration>) -> String {
    d.map_or_else(
        || "–".to_owned(),
        |d| format!("{:.2} ms", d.as_secs_f64() * 1000.0),
    )
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "idle",
        Phase::Ramp => "ramp",
        Phase::Settle => "settle",
        Phase::Steady => "steady",
        Phase::Drain => "drain",
    }
}
