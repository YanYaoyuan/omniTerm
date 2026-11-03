// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Host statistics from one read-only POSIX sh loop on the remote host, so nothing is
//! installed there. The script has no single quotes: it travels as `sh -c '<script>'`.
//! Remote output is untrusted: every value is checked, and anything unrecognized is left out.

use std::collections::HashMap;
use std::time::Instant;

pub const SAMPLE_SECONDS: u32 = 3;

pub fn script() -> String {
    format!(
        r#"export LC_ALL=C
os=$(uname -s)
while :; do
echo "@@begin $os"
case $os in
Linux)
echo @@cpu; head -n 1 /proc/stat
echo @@cores; grep -c "^cpu[0-9]" /proc/stat
echo @@mem; grep -E "^(MemTotal|MemAvailable|SwapTotal|SwapFree):" /proc/meminfo
echo @@load; cat /proc/loadavg
echo @@uptime; cat /proc/uptime
echo @@net; tail -n +3 /proc/net/dev
;;
Darwin)
echo @@cpupercent; top -l 1 -n 0 -s 0 | grep "CPU usage"
echo @@cores; sysctl -n hw.ncpu
echo @@memdarwin; sysctl -n hw.memsize hw.pagesize; vm_stat
echo @@load; sysctl -n vm.loadavg | tr -d "{{}}"
;;
esac
echo @@disk; df -Pk / 2>/dev/null | tail -n 1
echo @@nvidia
if command -v nvidia-smi >/dev/null 2>&1; then nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits 2>/dev/null; fi
echo @@amdgpu
for card in /sys/class/drm/card[0-9]*/device; do
if [ -r "$card/gpu_busy_percent" ]; then
echo "$(cat "$card/gpu_busy_percent" 2>/dev/null) $(cat "$card/mem_info_vram_used" 2>/dev/null) $(cat "$card/mem_info_vram_total" 2>/dev/null)"
fi
done
echo @@end
sleep {SAMPLE_SECONDS}
done
"#
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub used: f64,
    pub total: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub util: f64,
    pub mem: Option<Usage>,
    pub temp: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    pub os: String,
    pub cpu: Option<f64>,
    pub cores: Option<f64>,
    pub mem: Option<Usage>,
    pub swap: Option<Usage>,
    pub disk: Option<Usage>,
    pub load: Option<[f64; 3]>,
    pub uptime: Option<f64>,
    /// Bytes per second, received and sent.
    pub net: Option<(f64, f64)>,
    pub gpus: Vec<Gpu>,
    cpu_ticks: Option<(f64, f64)>,
    net_total: Option<(f64, f64)>,
}

fn number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|value| value.is_finite() && *value >= 0.0)
}

/// Parses the text of one `@@begin … @@end` block.
pub fn parse(text: &str) -> Sample {
    let mut sections: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut current: Option<&str> = None;
    let mut sample = Sample::default();
    for line in text.lines() {
        if let Some(marker) = line.strip_prefix("@@") {
            let (name, rest) = marker.split_once(' ').unwrap_or((marker, ""));
            if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            if name == "begin" {
                sample.os = rest.chars().take(40).collect();
            } else {
                current = Some(name);
                sections.entry(name).or_default();
            }
        } else if let Some(name) = current
            && !line.is_empty()
        {
            sections.entry(name).or_default().push(line);
        }
    }
    let first = |name: &str| sections.get(name).and_then(|lines| lines.first().copied());
    if let Some(cpu) = first("cpu") {
        let values: Vec<Option<f64>> = cpu.split_whitespace().skip(1).map(number).collect();
        if values.len() >= 4 && values.iter().all(Option::is_some) {
            let values: Vec<f64> = values.into_iter().flatten().collect();
            sample.cpu_ticks = Some((values.iter().take(8).sum(), values[3] + values.get(4).copied().unwrap_or(0.0)));
        }
    }
    if let Some(line) = first("cpupercent")
        && let Some(position) = line.find("% idle")
    {
        let start = line[..position].rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map(|i| i + 1).unwrap_or(0);
        if let Some(idle) = number(&line[start..position]) {
            sample.cpu = Some((100.0 - idle).clamp(0.0, 100.0));
        }
    }
    sample.cores = first("cores").and_then(number).filter(|cores| *cores > 0.0);
    let mut meminfo: HashMap<&str, f64> = HashMap::new();
    for line in sections.get("mem").into_iter().flatten() {
        if let Some((key, value)) = line.split_once(':')
            && let Some(kib) = value.split_whitespace().next().and_then(number)
        {
            meminfo.insert(key, kib * 1024.0);
        }
    }
    if let (Some(total), Some(available)) = (meminfo.get("MemTotal"), meminfo.get("MemAvailable")) {
        sample.mem = Some(Usage { total: *total, used: (total - available).max(0.0) });
    }
    if let Some(total) = meminfo.get("SwapTotal").filter(|total| **total > 0.0) {
        sample.swap = Some(Usage { total: *total, used: (total - meminfo.get("SwapFree").copied().unwrap_or(0.0)).max(0.0) });
    }
    if let Some(lines) = sections.get("memdarwin")
        && let (Some(total), Some(page)) = (lines.first().and_then(|l| number(l)), lines.get(1).and_then(|l| number(l)))
        && total > 0.0
    {
        let pages = |label: &str| lines.iter().find_map(|line| line.strip_prefix(label).and_then(|rest| rest.trim_start_matches(':').trim().trim_end_matches('.').parse::<f64>().ok())).unwrap_or(0.0);
        let used = (pages("Pages active") + pages("Pages wired down") + pages("Pages occupied by compressor")) * page;
        sample.mem = Some(Usage { total, used: used.min(total) });
    }
    if let Some(load) = first("load") {
        let values: Vec<f64> = load.split_whitespace().take(3).filter_map(number).collect();
        if values.len() == 3 {
            sample.load = Some([values[0], values[1], values[2]]);
        }
    }
    sample.uptime = first("uptime").and_then(|line| line.split_whitespace().next()).and_then(number);
    let (mut rx, mut tx, mut seen) = (0.0, 0.0, false);
    for line in sections.get("net").into_iter().flatten() {
        let Some((interface, rest)) = line.split_once(':') else { continue };
        if interface.trim() == "lo" {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if let (Some(received), Some(sent)) = (fields.first().and_then(|f| number(f)), fields.get(8).and_then(|f| number(f))) {
            rx += received;
            tx += sent;
            seen = true;
        }
    }
    if seen {
        sample.net_total = Some((rx, tx));
    }
    if let Some(disk) = first("disk") {
        let fields: Vec<&str> = disk.split_whitespace().collect();
        if fields.len() >= 6
            && let (Some(total), Some(used)) = (number(fields[1]), number(fields[2]))
            && total > 0.0
        {
            sample.disk = Some(Usage { total: total * 1024.0, used: used * 1024.0 });
        }
    }
    for line in sections.get("nvidia").into_iter().flatten() {
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let (Some(name), Some(util)) = (fields.first().filter(|name| !name.is_empty()), fields.get(1).and_then(|f| number(f))) else { continue };
        let mem = match (fields.get(2).and_then(|f| number(f)), fields.get(3).and_then(|f| number(f))) {
            (Some(used), Some(total)) => Some(Usage { used: used * 1_048_576.0, total: total * 1_048_576.0 }),
            _ => None,
        };
        sample.gpus.push(Gpu { name: name.chars().take(80).collect(), util, mem, temp: fields.get(4).and_then(|f| number(f)) });
    }
    for (index, line) in sections.get("amdgpu").into_iter().flatten().enumerate() {
        let values: Vec<Option<f64>> = line.split_whitespace().map(number).collect();
        if let Some(Some(util)) = values.first() {
            let mem = match (values.get(1).copied().flatten(), values.get(2).copied().flatten()) {
                (Some(used), Some(total)) => Some(Usage { used, total }),
                _ => None,
            };
            sample.gpus.push(Gpu { name: format!("AMD GPU {index}"), util: *util, mem, temp: None });
        }
    }
    sample.gpus.truncate(8);
    sample
}

/// CPU and network figures are counters, so each report compares two samples.
#[derive(Default)]
pub struct Tracker {
    previous: Option<(Option<(f64, f64)>, Option<(f64, f64)>, Instant)>,
}

impl Tracker {
    pub fn next(&mut self, mut sample: Sample) -> Sample {
        let now = Instant::now();
        if let Some((ticks, net, time)) = self.previous {
            if let (Some((total, idle)), Some((old_total, old_idle))) = (sample.cpu_ticks, ticks) {
                let elapsed = total - old_total;
                if elapsed > 0.0 {
                    sample.cpu = Some(((1.0 - (idle - old_idle) / elapsed) * 100.0).clamp(0.0, 100.0));
                }
            }
            let seconds = now.duration_since(time).as_secs_f64();
            if let (Some((rx, tx)), Some((old_rx, old_tx))) = (sample.net_total, net)
                && seconds > 0.0
            {
                sample.net = Some(((rx - old_rx).max(0.0) / seconds, (tx - old_tx).max(0.0) / seconds));
            }
        }
        self.previous = Some((sample.cpu_ticks, sample.net_total, now));
        sample
    }
}

/// Splits streamed output into complete samples, keeping the unfinished tail.
pub fn take_samples(buffer: &mut String) -> Vec<String> {
    let mut samples = Vec::new();
    while let Some(end) = buffer.find("@@end\n") {
        samples.push(buffer[..end].to_string());
        buffer.drain(..end + 6);
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX: &str = "@@begin Linux\n@@cpu\ncpu  100 0 100 800 0 0 0 0 0 0\n@@cores\n4\n@@mem\nMemTotal:       8000000 kB\nMemAvailable:   2000000 kB\nSwapTotal:      1000 kB\nSwapFree:       500 kB\n@@load\n0.50 0.40 0.30 1/200 1234\n@@uptime\n3600.5 100.0\n@@net\n  eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n    lo: 99 0 0 0 0 0 0 0 99 0 0 0 0 0 0 0\n@@disk\n/dev/sda1 1000 400 600 40% /\n@@nvidia\nRTX, 55, 1024, 8192, 61\n@@amdgpu\n";

    #[test]
    fn linux_samples_parse_every_section() {
        let sample = parse(LINUX);
        assert_eq!(sample.os, "Linux");
        assert_eq!(sample.cores, Some(4.0));
        assert_eq!(sample.mem, Some(Usage { total: 8_192_000_000.0, used: 6_144_000_000.0 }));
        assert_eq!(sample.swap.unwrap().used, 512_000.0);
        assert_eq!(sample.load, Some([0.5, 0.4, 0.3]));
        assert_eq!(sample.uptime, Some(3600.5));
        assert_eq!(sample.disk, Some(Usage { total: 1_024_000.0, used: 409_600.0 }));
        assert_eq!(sample.gpus[0].util, 55.0);
        assert_eq!(sample.net_total, Some((1000.0, 2000.0)), "loopback is left out");
    }

    #[test]
    fn rates_come_from_two_samples() {
        let mut tracker = Tracker::default();
        let first = tracker.next(parse(LINUX));
        assert_eq!(first.cpu, None);
        let busier = LINUX.replace("cpu  100 0 100 800", "cpu  200 0 200 900");
        let second = tracker.next(parse(&busier));
        assert!((second.cpu.unwrap() - 66.666).abs() < 0.1);
    }

    #[test]
    fn hostile_output_is_ignored_and_samples_split_on_their_end_marker() {
        let sample = parse("@@begin x\n@@load\n-1 abc 3\n@@disk\nnot enough\n");
        assert_eq!(sample.load, None);
        assert_eq!(sample.disk, None);
        let mut buffer = format!("{LINUX}@@end\n@@begin Linux\n@@cpu");
        assert_eq!(take_samples(&mut buffer).len(), 1);
        assert!(buffer.starts_with("@@begin Linux\n@@cpu"));
    }
}
