//! The host a run was measured on (Linux `/proc`); unknown fields read "unknown".

use std::thread::available_parallelism;

const UNKNOWN: &str = "unknown";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub cpu_model: String,
    pub logical_cores: usize,
    pub memory: String,
    pub kernel: String,
}

impl Machine {
    pub fn detect() -> Self {
        let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        Self {
            cpu_model: first_value(&read("/proc/cpuinfo"), "model name"),
            logical_cores: available_parallelism().map_or(0, usize::from),
            memory: first_value(&read("/proc/meminfo"), "MemTotal"),
            kernel: Some(read("/proc/sys/kernel/osrelease").trim().to_owned())
                .filter(|k| !k.is_empty())
                .unwrap_or_else(|| UNKNOWN.to_owned()),
        }
    }
}

/// The value of the first `key: value` line with this key.
fn first_value(text: &str, key: &str) -> String {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_owned())
        .unwrap_or_else(|| UNKNOWN.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_value_finds_the_key() {
        let cpuinfo = "processor\t: 0\nmodel name\t: AMD EPYC 7C13 64-Core Processor\n\
                       processor\t: 1\nmodel name\t: AMD EPYC 7C13 64-Core Processor\n";
        assert_eq!(
            first_value(cpuinfo, "model name"),
            "AMD EPYC 7C13 64-Core Processor"
        );
        assert_eq!(first_value(cpuinfo, "flags"), UNKNOWN);
    }
}
