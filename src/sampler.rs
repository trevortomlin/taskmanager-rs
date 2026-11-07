//! Collects system and per-process statistics via `sysinfo`.
//!
//! A [`Sampler`] owns the `sysinfo` state (CPU, disk and network figures are
//! deltas between consecutive refreshes) and produces plain-data [`Snapshot`]s.
//! Sampling runs on a background thread, so everything the UI shows — including
//! formatted text — is prepared here to keep per-frame rendering cheap.

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use gpui::SharedString;
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate,
    System, Uid, UpdateKind, Users,
};

use crate::format;

#[derive(Clone, Debug)]
pub struct ProcessRow {
    pub pid: u32,
    /// Seconds since the Unix epoch; together with `pid` this identifies a process.
    pub start_time: u64,
    pub name: SharedString,
    pub user: SharedString,
    /// Share of total CPU capacity across all cores, 0–100.
    pub cpu: f32,
    /// Resident memory in bytes.
    pub memory: u64,
    /// Combined disk read + write rate in bytes per second.
    pub disk: u64,
    pub display: RowDisplay,
}

/// Pre-rendered strings and styling inputs for a table row.
#[derive(Clone, Debug)]
pub struct RowDisplay {
    pub name_lower: String,
    pub user_lower: String,
    pub pid: SharedString,
    pub cpu: SharedString,
    pub memory: SharedString,
    pub disk: SharedString,
    pub initial: SharedString,
    /// Hue (0–1) of the name badge, stable per process name.
    pub badge_hue: f32,
}

impl ProcessRow {
    pub fn new(
        pid: u32,
        start_time: u64,
        name: SharedString,
        user: SharedString,
        cpu: f32,
        memory: u64,
        disk: u64,
    ) -> Self {
        let display = RowDisplay {
            name_lower: name.to_lowercase(),
            user_lower: user.to_lowercase(),
            pid: pid.to_string().into(),
            cpu: format::percent(cpu).into(),
            memory: format::bytes(memory).into(),
            disk: format::rate(disk).into(),
            initial: name
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().collect::<String>())
                .unwrap_or_else(|| "•".into())
                .into(),
            badge_hue: name_hue(&name),
        };
        Self {
            pid,
            start_time,
            name,
            user,
            cpu,
            memory,
            disk,
            display,
        }
    }
}

fn name_hue(name: &str) -> f32 {
    let hash = name
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    (hash % 360) as f32 / 360.0
}

#[derive(Clone, Debug)]
pub struct NetworkInterface {
    pub name: SharedString,
    /// Bytes per second.
    pub rx: u64,
    pub tx: u64,
    /// Bytes since boot.
    pub total_rx: u64,
    pub total_tx: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub processes: Vec<ProcessRow>,
    /// Overall CPU usage, 0–100.
    pub cpu: f32,
    /// Per logical core usage, 0–100.
    pub cpu_cores: Vec<f32>,
    pub cpu_brand: SharedString,
    /// Average core frequency in MHz.
    pub cpu_frequency: u64,
    pub memory_used: u64,
    pub memory_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    /// Total network receive/transmit rate in bytes per second.
    pub net_rx: u64,
    pub net_tx: u64,
    pub interfaces: Vec<NetworkInterface>,
    /// Total disk read/write rate across all processes, bytes per second.
    pub disk_read: u64,
    pub disk_write: u64,
    pub uptime_secs: u64,
}

impl Snapshot {
    pub fn memory_fraction(&self) -> f32 {
        fraction(self.memory_used, self.memory_total)
    }

    pub fn swap_fraction(&self) -> f32 {
        fraction(self.swap_used, self.swap_total)
    }
}

fn fraction(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        used as f32 / total as f32
    }
}

pub struct Sampler {
    sys: System,
    networks: Option<Networks>,
    users: UserNames,
    /// Processes seen in the previous sample, keyed by PID, valued by start time.
    known_processes: HashMap<u32, u64>,
    known_interfaces: HashSet<String>,
    last_refresh: Instant,
}

impl Sampler {
    pub fn new() -> Self {
        let mut sampler = Self {
            sys: System::new().expect("sysinfo is not supported on this platform"),
            networks: Networks::new_with_refreshed_list().ok(),
            users: UserNames::default(),
            known_processes: HashMap::new(),
            known_interfaces: HashSet::new(),
            last_refresh: Instant::now(),
        };
        // Prime the delta-based counters so the first real sample has a baseline.
        sampler.sample();
        sampler
    }

    fn refresh(&mut self) -> f64 {
        self.sys
            .refresh_cpu_specifics(CpuRefreshKind::nothing().with_usage().with_frequency());
        self.sys
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram().with_swap());
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            // `nothing()` still enumerates every thread as its own entry; we
            // only show processes, so skip the per-thread /proc walk.
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_user(UpdateKind::OnlyIfNotSet),
        );
        if let Some(networks) = &mut self.networks {
            networks.refresh(true);
        }
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refresh).as_secs_f64();
        self.last_refresh = now;
        elapsed.max(0.001)
    }

    pub fn sample(&mut self) -> Snapshot {
        let elapsed = self.refresh();
        let per_sec = |bytes: u64| (bytes as f64 / elapsed) as u64;
        let cpus = self.sys.cpus();
        let cpu_count = cpus.len().max(1);

        let mut disk_read = 0;
        let mut disk_write = 0;
        let mut known = HashMap::with_capacity(self.known_processes.len());
        let mut processes = Vec::with_capacity(self.known_processes.len());
        // Hide kernel and userland threads; show only real processes.
        for p in self
            .sys
            .processes()
            .values()
            .filter(|p| p.thread_kind().is_none())
        {
            let pid = p.pid().as_u32();
            let start_time = p.start_time();
            known.insert(pid, start_time);
            // sysinfo reports a new process's lifetime I/O as its first delta, so
            // only trust disk activity once we've seen the process before.
            let disk = if self.known_processes.get(&pid) == Some(&start_time) {
                let usage = p.disk_usage();
                disk_read += usage.read_bytes;
                disk_write += usage.written_bytes;
                per_sec(usage.read_bytes + usage.written_bytes)
            } else {
                0
            };
            processes.push(ProcessRow::new(
                pid,
                start_time,
                p.name().to_string_lossy().into_owned().into(),
                p.user_id()
                    .map(|uid| self.users.name(uid))
                    .unwrap_or_default(),
                p.cpu_usage() / cpu_count as f32,
                p.memory(),
                disk,
            ));
        }
        self.known_processes = known;

        let mut interfaces = Vec::new();
        let mut known_interfaces = HashSet::new();
        for (name, data) in self.networks.iter().flat_map(|n| n.iter()) {
            let name = name.to_string_lossy();
            if !is_physical_interface(&name) {
                continue;
            }
            // Same first-delta problem as disk I/O for interfaces that just appeared.
            let seen = self.known_interfaces.contains(name.as_ref());
            known_interfaces.insert(name.to_string());
            interfaces.push(NetworkInterface {
                name: name.into_owned().into(),
                rx: if seen { per_sec(data.received()) } else { 0 },
                tx: if seen { per_sec(data.transmitted()) } else { 0 },
                total_rx: data.total_received(),
                total_tx: data.total_transmitted(),
            });
        }
        self.known_interfaces = known_interfaces;
        interfaces.sort_by(|a, b| {
            (b.total_rx + b.total_tx)
                .cmp(&(a.total_rx + a.total_tx))
                .then_with(|| a.name.cmp(&b.name))
        });

        Snapshot {
            processes,
            cpu: self.sys.global_cpu_usage(),
            cpu_cores: cpus.iter().map(|c| c.usage()).collect(),
            cpu_brand: cpus
                .first()
                .map(|c| c.brand().trim().to_owned())
                .unwrap_or_default()
                .into(),
            cpu_frequency: cpus.iter().map(|c| c.frequency()).sum::<u64>() / cpu_count as u64,
            memory_used: self.sys.used_memory(),
            memory_total: self.sys.total_memory(),
            swap_used: self.sys.used_swap(),
            swap_total: self.sys.total_swap(),
            net_rx: interfaces.iter().map(|i| i.rx).sum(),
            net_tx: interfaces.iter().map(|i| i.tx).sum(),
            interfaces,
            disk_read: per_sec(disk_read),
            disk_write: per_sec(disk_write),
            uptime_secs: System::uptime().unwrap_or(0),
        }
    }
}

/// Resolves user IDs to names, reloading the system user list when an unknown
/// ID shows up (e.g. an account created after launch).
#[derive(Default)]
struct UserNames {
    names: HashMap<Uid, SharedString>,
    /// UIDs that were still unknown after a reload; avoids reloading every sample.
    unresolved: HashSet<Uid>,
}

impl UserNames {
    fn name(&mut self, uid: &Uid) -> SharedString {
        if let Some(name) = self.names.get(uid) {
            return name.clone();
        }
        if !self.unresolved.contains(uid) {
            self.reload();
            if let Some(name) = self.names.get(uid) {
                return name.clone();
            }
            self.unresolved.insert(uid.clone());
        }
        SharedString::default()
    }

    fn reload(&mut self) {
        let Ok(users) = Users::new_with_refreshed_list() else {
            return;
        };
        self.names = users
            .iter()
            .map(|u| (u.id().clone(), SharedString::from(u.name().to_owned())))
            .collect();
        self.unresolved.clear();
    }
}

/// Looks up a process's start time directly, for confirming its identity.
pub fn process_start_time(pid: u32) -> Option<u64> {
    let mut sys = System::new().ok()?;
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing(),
    );
    sys.process(pid).map(|p| p.start_time())
}

/// Loopback and container/VM bridge traffic would double-count what already
/// flows through the real NICs, so leave them out of the totals.
fn is_physical_interface(name: &str) -> bool {
    const VIRTUAL_PREFIXES: [&str; 6] = ["veth", "docker", "br-", "virbr", "vnet", "podman"];
    name != "lo" && !VIRTUAL_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_virtual_interfaces() {
        assert!(is_physical_interface("enp5s0"));
        assert!(is_physical_interface("wlan0"));
        assert!(is_physical_interface("lowpan0"));
        assert!(!is_physical_interface("lo"));
        assert!(!is_physical_interface("veth12ab"));
        assert!(!is_physical_interface("docker0"));
    }

    #[test]
    fn row_precomputes_display_text() {
        let row = ProcessRow::new(42, 0, "Firefox".into(), "Trevor".into(), 2.54, 2048, 0);
        assert_eq!(row.display.name_lower, "firefox");
        assert_eq!(row.display.user_lower, "trevor");
        assert_eq!(row.display.pid.as_ref(), "42");
        assert_eq!(row.display.cpu.as_ref(), "2.5%");
        assert_eq!(row.display.memory.as_ref(), "2.0 KB");
        assert_eq!(row.display.disk.as_ref(), "—");
        assert_eq!(row.display.initial.as_ref(), "F");
    }

    #[test]
    fn unseen_processes_report_no_disk_activity() {
        let mut sampler = Sampler::new();
        let me = std::process::id();
        let snapshot = sampler.sample();
        assert!(snapshot.processes.iter().any(|p| p.pid == me));
        // Forget everything: every process is now "new" and must report zero I/O
        // rather than its lifetime totals.
        sampler.known_processes.clear();
        let snapshot = sampler.sample();
        assert!(snapshot.processes.iter().all(|p| p.disk == 0));
        assert_eq!(snapshot.disk_read + snapshot.disk_write, 0);
    }

    #[test]
    fn start_time_lookup_matches_sampler() {
        let mut sampler = Sampler::new();
        let me = std::process::id();
        let row_start = sampler
            .sample()
            .processes
            .into_iter()
            .find(|p| p.pid == me)
            .map(|p| p.start_time);
        assert!(row_start.is_some());
        assert_eq!(row_start, process_start_time(me));
    }
}
