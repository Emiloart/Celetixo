//! Observation helpers for the explicitly invoked benchmark programs.
use serde_json::{Value, json};
use std::time::Instant;

pub fn micros(start: Instant) -> u64 {
    start.elapsed().as_micros().try_into().unwrap_or(u64::MAX)
}

/// Linux process observations, not allocation accounting for a particular call.
pub fn memory() -> Value {
    let status = std::fs::read_to_string("/proc/self/status").expect("Linux process status");
    let kib = |key| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v * 1024)
    };
    json!({"current_rss_bytes":kib("VmRSS:"),"process_lifetime_high_water_rss_bytes":kib("VmHWM:")})
}

pub fn environment() -> Value {
    let cpu = std::fs::read_to_string("/proc/cpuinfo").expect("Linux CPU identity");
    let version = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .expect("reviewed Rust toolchain");
    assert!(version.status.success());
    json!({"kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),
        "architecture":std::env::consts::ARCH,"os":std::env::consts::OS,
        "cpu":cpu.lines().find(|line|line.starts_with("model name")),
        "logical_cpus":std::thread::available_parallelism().unwrap().get(),
        "rustc":String::from_utf8(version.stdout).unwrap().trim(),
        "build_profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "cold_cache_claim":false,"cache_eviction_performed":false,
        "memory_scope":"/proc/self/status observations; high water belongs to the whole benchmark process"})
}
