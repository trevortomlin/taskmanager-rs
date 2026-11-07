//! Collects system and per-process statistics via `sysinfo`.
//!
//! A [`Sampler`] owns the `sysinfo` state (CPU, disk and network figures are
//! deltas between consecutive refreshes) and produces plain-data [`Snapshot`]s
//! that the UI can render without touching `sysinfo` directly.

use std::time::Instant;

use gpui::SharedString;
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate, System,
    UpdateKind, Users,
};

#[derive(Clone, Debug)]
pub struct ProcessRow {
    pub pid: u32,
    pub name: SharedString,
    pub user: SharedString,
    /// Share of total CPU capacity across all cores, 0–100.
    pub cpu: f32,
    /// Resident memory in bytes.
    pub memory: u64,
    /// Combined disk read + write rate in bytes per second.
    pub disk: u64,
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

pub struct Sampler {
    sys: System,
    networks: Option<Networks>,
    users: Option<Users>,
    last_refresh: Instant,
}

impl Sampler {
    pub fn new() -> Self {
        let mut sampler = Self {
            sys: System::new().expect("sysinfo is not supported on this platform"),
            networks: Networks::new_with_refreshed_list().ok(),
            users: Users::new_with_refreshed_list().ok(),
            last_refresh: Instant::now(),
        };
        // Prime the delta-based counters so the next sample has a baseline.
        sampler.refresh();
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
            ProcessRefreshKind::nothing()
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
        elapsed
    }

    pub fn sample(&mut self) -> Snapshot {
        let elapsed = self.refresh().max(0.001);
        let per_sec = |bytes: u64| (bytes as f64 / elapsed) as u64;
        let cpus = self.sys.cpus();
        let cpu_count = cpus.len().max(1);

        let mut disk_read = 0;
        let mut disk_write = 0;
        let processes = self
            .sys
            .processes()
            .values()
            // Hide kernel and userland threads; show only real processes.
            .filter(|p| p.thread_kind().is_none())
            .map(|p| {
                let disk = p.disk_usage();
                disk_read += disk.read_bytes;
                disk_write += disk.written_bytes;
                let user = p
                    .user_id()
                    .and_then(|uid| self.users.as_ref()?.get_user_by_id(uid))
                    .map(|u| SharedString::from(u.name().to_owned()))
                    .unwrap_or_default();
                ProcessRow {
                    pid: p.pid().as_u32(),
                    name: p.name().to_string_lossy().into_owned().into(),
                    user,
                    cpu: p.cpu_usage() / cpu_count as f32,
                    memory: p.memory(),
                    disk: per_sec(disk.read_bytes + disk.written_bytes),
                }
            })
            .collect();

        let mut interfaces: Vec<NetworkInterface> = self
            .networks
            .iter()
            .flat_map(|n| n.iter())
            .filter(|(name, _)| is_physical_interface(&name.to_string_lossy()))
            .map(|(name, data)| NetworkInterface {
                name: name.to_string_lossy().into_owned().into(),
                rx: per_sec(data.received()),
                tx: per_sec(data.transmitted()),
                total_rx: data.total_received(),
                total_tx: data.total_transmitted(),
            })
            .collect();
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

/// Loopback and container/VM bridge traffic would double-count what already
/// flows through the real NICs, so leave them out of the totals.
fn is_physical_interface(name: &str) -> bool {
    const VIRTUAL_PREFIXES: [&str; 7] = ["lo", "veth", "docker", "br-", "virbr", "vnet", "podman"];
    !VIRTUAL_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::is_physical_interface;

    #[test]
    fn filters_virtual_interfaces() {
        assert!(is_physical_interface("enp5s0"));
        assert!(is_physical_interface("wlan0"));
        assert!(!is_physical_interface("lo"));
        assert!(!is_physical_interface("veth12ab"));
        assert!(!is_physical_interface("docker0"));
    }
}
