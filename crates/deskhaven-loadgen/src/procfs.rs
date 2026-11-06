//! CPU time, RSS and storage writes of one process, from `/proc/<pid>` (Linux).
//!
//! Only the server's own counters: the load generator runs on the same host, so host-wide
//! figures would count the simulated clients too. `utime + stime` of `/proc/<pid>/stat`
//! cover every thread of the process.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::LoadgenError;

/// One reading of a process's counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcSample {
    pub at: Instant,
    /// User + system CPU time, in clock ticks.
    pub cpu_ticks: u64,
    /// Resident set size (`VmRSS`).
    pub rss_bytes: u64,
    /// Bytes the process caused to be sent to storage (`/proc/<pid>/io`); `None` where the
    /// kernel does not expose it.
    pub write_bytes: Option<u64>,
}

/// `utime + stime` (fields 14 and 15) of a `/proc/<pid>/stat` line. The command name (field
/// 2) is parenthesized and may contain spaces, so fields are counted after its last `)`.
pub fn parse_cpu_ticks(stat: &str) -> Option<u64> {
    /// Fields 14 (utime) and 15 (stime), counted from field 3 (state), the first after `)`.
    const UTIME: usize = 14 - 3;
    let (_, after_name) = stat.rsplit_once(')')?;
    let mut fields = after_name.split_whitespace().skip(UTIME);
    let utime: u64 = fields.next()?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    utime.checked_add(stime)
}

/// `VmRSS` of `/proc/<pid>/status` (reported in kB, i.e. KiB), in bytes.
pub fn parse_rss_bytes(status: &str) -> Option<u64> {
    let kib: u64 = field(status, "VmRSS:")?
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    kib.checked_mul(1024)
}

/// `write_bytes` of `/proc/<pid>/io`.
pub fn parse_write_bytes(io: &str) -> Option<u64> {
    field(io, "write_bytes:")?.parse().ok()
}

/// The trimmed rest of the line starting with `key`.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .map(str::trim)
}

/// Average cores used between two samples: CPU seconds over wall seconds.
pub fn cores_between(earlier: &ProcSample, later: &ProcSample, ticks_per_second: u64) -> f64 {
    let wall = later.at.saturating_duration_since(earlier.at).as_secs_f64();
    if wall <= 0.0 || ticks_per_second == 0 {
        return 0.0;
    }
    let ticks = later.cpu_ticks.saturating_sub(earlier.cpu_ticks);
    ticks as f64 / ticks_per_second as f64 / wall
}

/// The kernel's `USER_HZ`, the unit of `/proc/<pid>/stat` CPU times.
#[cfg(unix)]
pub fn ticks_per_second() -> u64 {
    rustix::param::clock_ticks_per_second()
}

/// Elsewhere there is no `/proc`; every sample fails before this matters.
#[cfg(not(unix))]
pub fn ticks_per_second() -> u64 {
    100
}

/// Reads the counters of process `pid` now.
pub fn sample(pid: u32) -> Result<ProcSample, LoadgenError> {
    let at = Instant::now();
    let stat = read(pid, "stat")?;
    let status = read(pid, "status")?;
    let malformed = |file: &'static str| LoadgenError::Proc {
        pid,
        file,
        source: std::io::Error::other("unexpected format"),
    };
    Ok(ProcSample {
        at,
        cpu_ticks: parse_cpu_ticks(&stat).ok_or_else(|| malformed("stat"))?,
        rss_bytes: parse_rss_bytes(&status).ok_or_else(|| malformed("status"))?,
        // Absent without CONFIG_TASK_IO_ACCOUNTING; optional evidence only.
        write_bytes: read(pid, "io").ok().as_deref().and_then(parse_write_bytes),
    })
}

fn read(pid: u32, file: &'static str) -> Result<String, LoadgenError> {
    let path = PathBuf::from(format!("/proc/{pid}/{file}"));
    std::fs::read_to_string(path).map_err(|source| LoadgenError::Proc { pid, file, source })
}

/// The largest average over any `window` of consecutive samples (at least two samples
/// apart), e.g. the busiest 10 s of a phase.
pub fn peak_cores(samples: &[ProcSample], window: Duration, ticks_per_second: u64) -> Option<f64> {
    samples
        .iter()
        .enumerate()
        .filter_map(|(i, start)| {
            let end = samples[i + 1..]
                .iter()
                .find(|s| s.at.saturating_duration_since(start.at) >= window)?;
            Some(cores_between(start, end, ticks_per_second))
        })
        .reduce(f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "4242 (deskhaven serve) S 1 4242 4242 0 -1 4194560 3000 0 0 0 \
                        1500 250 0 0 20 0 9 0 100 123456789 2048 18446744073709551615";

    fn at(base: Instant, secs: u64, cpu_ticks: u64) -> ProcSample {
        ProcSample {
            at: base + Duration::from_secs(secs),
            cpu_ticks,
            rss_bytes: 0,
            write_bytes: None,
        }
    }

    #[test]
    fn cpu_ticks_are_utime_plus_stime_counted_after_the_command_name() {
        assert_eq!(parse_cpu_ticks(STAT), Some(1750));
        let tricky = STAT.replace("(deskhaven serve)", "(a) b ) c)");
        assert_eq!(parse_cpu_ticks(&tricky), Some(1750));
        assert_eq!(parse_cpu_ticks("4242 (x) S 1 2"), None);
    }

    #[test]
    fn rss_is_read_from_vmrss_in_kib() {
        let status = "Name:\tdeskhaven\nVmPeak:\t  99999 kB\nVmRSS:\t   23456 kB\nThreads:\t9\n";
        assert_eq!(parse_rss_bytes(status), Some(23_456 * 1024));
        assert_eq!(parse_rss_bytes("Name:\tx\n"), None);
    }

    #[test]
    fn write_bytes_is_read_from_io() {
        let io = "rchar: 10\nwchar: 20\nsyscr: 1\nsyscw: 2\nread_bytes: 0\nwrite_bytes: 81920\ncancelled_write_bytes: 0\n";
        assert_eq!(parse_write_bytes(io), Some(81_920));
        assert_eq!(parse_write_bytes("rchar: 10\n"), None);
    }

    #[test]
    fn cores_are_cpu_seconds_over_wall_seconds() {
        // Arrange: 150 ticks at 100 Hz over 3 s = 0.5 cores.
        let base = Instant::now();

        // Act + Assert
        let cores = cores_between(&at(base, 0, 1000), &at(base, 3, 1150), 100);
        assert!((cores - 0.5).abs() < 1e-9, "{cores}");
        assert!(cores_between(&at(base, 3, 1150), &at(base, 3, 1150), 100).abs() < 1e-9);
    }

    #[test]
    fn peak_cores_is_the_busiest_window() {
        // Arrange: 1 s samples; a 2-core burst between t=2 and t=4.
        let base = Instant::now();
        let ticks = [0, 10, 20, 220, 420, 430];
        let samples: Vec<ProcSample> = ticks
            .iter()
            .enumerate()
            .map(|(i, &t)| at(base, u64::try_from(i).unwrap(), t))
            .collect();

        // Act
        let peak = peak_cores(&samples, Duration::from_secs(2), 100).unwrap();

        // Assert
        assert!((peak - 2.0).abs() < 1e-9, "{peak}");
        assert_eq!(peak_cores(&samples[..1], Duration::from_secs(2), 100), None);
    }
}
