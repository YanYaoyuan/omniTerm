// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! What a monitor reads, from the host itself and never from a terminal screen: one read-only
//! POSIX sh script (PowerShell on local Windows), so nothing is installed remotely. Scripts
//! carry no single quotes, because they travel as `sh -c '<script>'`, and every value in them
//! is validated here first.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const MAX_LOG_BYTES: u64 = 64 * 1024;
const LOG_MARK_BYTES: u64 = 256;
const MAX_REGION_BYTES: u32 = 4096;
const MAX_PROCESSES: usize = 5000;
const TICKS_PER_SECOND: f64 = 10_000_000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub field: String,
    pub offset: u32,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Target {
    Process {
        #[serde(skip_serializing_if = "Option::is_none")]
        pid: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Thread { pid: u64, tid: u64 },
    Log { path: String },
    SharedMemory { name: String, access: String, layout: Vec<Field> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Mac,
    Windows,
}

impl Platform {
    pub fn local() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        }
    }
}

fn width(kind: &str) -> Option<u32> {
    match kind {
        "u8" | "i8" => Some(1),
        "u16" | "i16" => Some(2),
        "u32" | "i32" | "f32" => Some(4),
        "u64" | "i64" | "f64" => Some(8),
        _ => None,
    }
}

fn name_ok(value: &str) -> bool {
    (1..=128).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric() || "_.+@-".contains(c))
}

/// Letters and digits of any language, but no quotes of any kind: PowerShell also treats curly
/// quotes as quotes, and these paths sit inside quoted strings.
fn path_ok(value: &str, windows: bool) -> bool {
    let allowed = |c: char, extra: &str| c.is_alphanumeric() || extra.contains(c);
    if value.contains("..") {
        return false;
    }
    if windows {
        let bytes = value.as_bytes();
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' && value.chars().count() <= 4003 && value[3..].chars().all(|c| allowed(c, "_ .\\+@()~#,-"))
    } else {
        value.starts_with('/') && value.chars().count() <= 4096 && value[1..].chars().all(|c| allowed(c, "_ ./+@~#,-"))
    }
}

/// Why a host cannot read this kind of target, if it cannot.
pub fn support(kind: &str, platform: Platform) -> Option<&'static str> {
    match (platform, kind) {
        (Platform::Windows, "sharedMemory") => Some("Windows 本机暂不支持共享内存监控。"),
        (Platform::Mac, "thread" | "sharedMemory") => Some("macOS 暂不支持这类监控。"),
        _ => None,
    }
}

pub fn validate(input: &Value, platform: Platform) -> Result<Target, String> {
    let kind = input["kind"].as_str().unwrap_or_default();
    if let Some(reason) = support(kind, platform) {
        return Err(reason.into());
    }
    let integer = |value: &Value, label: &str| value.as_u64().filter(|v| *v <= 9_007_199_254_740_991).ok_or_else(|| format!("{label}格式不正确。"));
    match kind {
        "process" => {
            if !input["pid"].is_null() {
                return Ok(Target::Process { pid: Some(integer(&input["pid"], "进程 ID")?), name: None });
            }
            let name = input["name"].as_str().unwrap_or_default();
            if !name_ok(name) {
                return Err("请填写进程 ID 或进程名。".into());
            }
            Ok(Target::Process { pid: None, name: Some(name.into()) })
        }
        "thread" => Ok(Target::Thread { pid: integer(&input["pid"], "进程 ID")?, tid: integer(&input["tid"], "线程 ID")? }),
        "log" => {
            let path = input["path"].as_str().unwrap_or_default();
            if !path_ok(path, platform == Platform::Windows) {
                return Err("日志路径必须是绝对路径，且只能包含常见文件名字符。".into());
            }
            Ok(Target::Log { path: path.into() })
        }
        "sharedMemory" => {
            if input["access"].as_str() != Some("posix-shm") {
                return Err("共享内存目前只支持只读的 POSIX 共享内存（/dev/shm）。".into());
            }
            let name = input["name"].as_str().unwrap_or_default();
            if !name_ok(name) {
                return Err("请填写共享内存区域名称。".into());
            }
            let layout = input["layout"].as_array().filter(|layout| !layout.is_empty() && layout.len() <= 32).ok_or("请填写共享内存布局。")?;
            let layout = layout
                .iter()
                .map(|item| {
                    let field = item["field"].as_str().unwrap_or_default();
                    let kind = item["type"].as_str().unwrap_or_default();
                    let size = width(kind).filter(|_| (1..=64).contains(&field.len()) && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).ok_or("共享内存布局的字段或类型不正确。")?;
                    let offset = integer(&item["offset"], "偏移量")?;
                    if offset + size as u64 > MAX_REGION_BYTES as u64 {
                        return Err(format!("共享内存布局不能超过 {MAX_REGION_BYTES} 字节。"));
                    }
                    Ok(Field { field: field.into(), offset: offset as u32, kind: kind.into() })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Target::SharedMemory { name: name.into(), access: "posix-shm".into(), layout })
        }
        _ => Err("未知的监控目标。".into()),
    }
}

/// A saved mark comes back from a remote host and goes into a script, so it is checked first.
fn saved_mark(mark: Option<&str>) -> Option<&str> {
    mark.filter(|mark| (1..=128).contains(&mark.len()) && mark.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

const UTF8_OUTPUT: &str = "$ErrorActionPreference = 'SilentlyContinue'\n[Console]::OutputEncoding = [Text.Encoding]::UTF8\n";

/// The volume and file ID of the open handle, the Windows counterpart of an inode. Defined at
/// run time, because Add-Type would start a compiler on every sample.
const WINDOWS_FILE_ID: &str = r#"function Get-FileId($handle) {
  try {
    $name = New-Object Reflection.AssemblyName 'OmniTermFileId'
    $assembly = [Reflection.Emit.AssemblyBuilder]::DefineDynamicAssembly($name, 'Run')
    $type = $assembly.DefineDynamicModule('Native').DefineType('FileId', 'Public,Class,Abstract,Sealed')
    $parameters = [Type[]]@([Microsoft.Win32.SafeHandles.SafeFileHandle], [int], [byte[]], [int])
    $method = $type.DefinePInvokeMethod('GetFileInformationByHandleEx', 'kernel32.dll', [Reflection.MethodAttributes]'Public,Static,PinvokeImpl', [Reflection.CallingConventions]'Standard', [bool], $parameters, [Runtime.InteropServices.CallingConvention]'Winapi', [Runtime.InteropServices.CharSet]'Auto')
    $method.SetImplementationFlags($method.GetMethodImplementationFlags() -bor [Reflection.MethodImplAttributes]::PreserveSig)
    $native = $type.CreateType()
    $info = New-Object byte[] 24
    if (-not $native::GetFileInformationByHandleEx($handle, 18, $info, 24)) { return 'none' }
    if ([BitConverter]::ToUInt64($info, 8) -eq 0 -and [BitConverter]::ToUInt64($info, 16) -eq 0) { return 'none' }
    return [BitConverter]::ToString($info).Replace('-', '')
  } catch { return 'none' }
}
"#;

/// The probe's memory between samples: where a log was read to, and which file that was.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub offset: Option<u64>,
    pub mark: Option<String>,
}

pub fn script(target: &Target, state: &State, platform: Platform) -> Result<String, String> {
    if platform == Platform::Windows {
        return Ok(match target {
            Target::Process { pid, name } => {
                let find = match pid {
                    Some(pid) => format!("Get-Process -Id {pid}"),
                    None => format!("Get-Process -Name '{}' | Select-Object -First 1", name.as_deref().unwrap_or_default().trim_end_matches(".exe")),
                };
                format!("{UTF8_OUTPUT}$p = {find}\nif (-not $p) {{ 'exists=0'; exit 0 }}\n'exists=1'\n\"pid=$($p.Id)\"\n\"cpuSeconds=$([math]::Round($p.CPU, 3))\"\n\"memoryKb=$([int64]($p.WorkingSet64 / 1024))\"\n\"command=$($p.ProcessName)\"\n\"threads=$($p.Threads.Count)\"\n")
            }
            Target::Thread { pid, tid } => format!("{UTF8_OUTPUT}$t = (Get-Process -Id {pid}).Threads | Where-Object {{ $_.Id -eq {tid} }}\nif (-not $t) {{ 'exists=0'; exit 0 }}\n'exists=1'\n\"cpuSeconds=$([math]::Round($t.TotalProcessorTime.TotalSeconds, 3))\"\n"),
            Target::Log { path } => {
                let start = state.offset.map(|offset| offset.to_string()).unwrap_or_else(|| "$size".into());
                let check = saved_mark(state.mark.as_deref()).map(|mark| format!(" -or (Get-Mark $start) -ne '{mark}'")).unwrap_or_default();
                format!(
                    "{UTF8_OUTPUT}{WINDOWS_FILE_ID}$path = '{path}'\nif (-not (Test-Path -LiteralPath $path -PathType Leaf)) {{ 'exists=0'; exit 0 }}\n'exists=1'\n$file = [IO.File]::Open($path, 'Open', 'Read', 'ReadWrite,Delete')\n$id = Get-FileId $file.SafeFileHandle\nif ($id -eq 'none') {{ 'identity=none' }}\nfunction Get-Mark($end) {{\n  $length = [Math]::Min({LOG_MARK_BYTES}, $end)\n  $window = New-Object byte[] (2 * $length)\n  [void]$file.Seek(0, 'Begin')\n  [void]$file.Read($window, 0, $length)\n  [void]$file.Seek($end - $length, 'Begin')\n  [void]$file.Read($window, $length, $length)\n  $id + '-' + [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($window)).Replace('-', '')\n}}\n$size = $file.Length\n\"size=$size\"\n$start = {start}\nif ($start -gt $size{check}) {{ $start = 0 }}\n[void]$file.Seek($start, 'Begin')\n$bytes = New-Object byte[] ([Math]::Min({MAX_LOG_BYTES}, $size - $start))\n$read = $file.Read($bytes, 0, $bytes.Length)\n$next = $start + $read\n$mark = Get-Mark $next\n$file.Close()\n\"offset=$start\"\n\"next=$next\"\n\"mark=$mark\"\n'@@text'\n[Console]::Out.Write([Text.Encoding]::UTF8.GetString($bytes, 0, $read))\n"
                )
            }
            Target::SharedMemory { .. } => return Err(support("sharedMemory", platform).unwrap_or_default().into()),
        });
    }
    let script = match target {
        Target::Process { pid, name } => {
            let find = match pid {
                Some(pid) => format!("pid={pid}"),
                None => format!("pid=$(pgrep -x {} 2>/dev/null | head -n 1)", name.as_deref().unwrap_or_default()),
            };
            format!("export LC_ALL=C\n{find}\nif [ -z \"$pid\" ] || ! kill -0 \"$pid\" 2>/dev/null && ! ps -p \"$pid\" >/dev/null 2>&1; then echo exists=0; exit 0; fi\necho exists=1\necho pid=$pid\nps -p \"$pid\" -o pcpu=,rss=,comm= | head -n 1 | {{ read -r cpu rss comm; echo cpu=$cpu; echo memoryKb=$rss; echo command=$comm; }}\nif [ -r \"/proc/$pid/status\" ]; then echo threads=$(grep \"^Threads:\" \"/proc/$pid/status\" | tr -cd 0-9); fi\n")
        }
        Target::Thread { pid, tid } => format!("export LC_ALL=C\nif [ ! -d \"/proc/{pid}/task/{tid}\" ]; then echo exists=0; exit 0; fi\necho exists=1\nps -L -p {pid} -o tid=,pcpu= | while read -r id cpu; do if [ \"$id\" = \"{tid}\" ]; then echo cpu=$cpu; fi; done\n"),
        // An offset only means something in the file it was taken from. The mark names that
        // file: its inode, which a rename changes, and a checksum of the bytes at its start and
        // before the offset, which also catch a reused inode and a rewrite in place.
        Target::Log { path } => {
            let start = state.offset.map(|offset| offset.to_string()).unwrap_or_else(|| "$size".into());
            let check = saved_mark(state.mark.as_deref()).map(|mark| format!(" || [ \"$(mark \"$start\")\" != \"{mark}\" ]")).unwrap_or_default();
            format!("export LC_ALL=C\nfile=\"{path}\"\nif [ ! -r \"$file\" ]; then echo exists=0; exit 0; fi\necho exists=1\ninode=$(ls -idL \"$file\" | {{ read -r id rest; echo \"$id\"; }})\nmark() {{\n  length=$1\n  if [ \"$length\" -gt {LOG_MARK_BYTES} ]; then length={LOG_MARK_BYTES}; fi\n  if [ \"$length\" -gt 0 ]; then head -c \"$length\" \"$file\"; tail -c +$(($1 - length + 1)) \"$file\" | head -c \"$length\"; fi | cksum | {{ read -r crc rest; echo \"$inode-$crc\"; }}\n}}\nsize=$(wc -c < \"$file\" | tr -d \" \")\necho size=$size\nstart={start}\nif [ \"$start\" -gt \"$size\" ]{check}; then start=0; fi\ncount=$((size - start))\nif [ \"$count\" -gt {MAX_LOG_BYTES} ]; then count={MAX_LOG_BYTES}; fi\nnext=$((start + count))\necho offset=$start\necho next=$next\necho mark=$(mark \"$next\")\necho @@text\nif [ \"$count\" -gt 0 ]; then tail -c +$((start + 1)) \"$file\" | head -c \"$count\"; fi\n")
        }
        Target::SharedMemory { name, layout, .. } => {
            let length = layout.iter().map(|field| field.offset + width(&field.kind).unwrap_or(1)).max().unwrap_or(1);
            format!("export LC_ALL=C\nif [ ! -r \"/dev/shm/{name}\" ]; then echo exists=0; exit 0; fi\necho exists=1\necho @@bytes\nod -An -v -tx1 -N {length} \"/dev/shm/{name}\" | tr -d \" \\n\"\n")
        }
    };
    if script.contains('\'') {
        return Err("Probe scripts must not contain single quotes.".into());
    }
    Ok(script)
}

fn read_field(bytes: &[u8], field: &Field) -> Value {
    let at = field.offset as usize;
    let size = width(&field.kind).unwrap_or(1) as usize;
    let Some(slice) = bytes.get(at..at + size) else { return Value::Null };
    let number = match field.kind.as_str() {
        "u8" => slice[0] as f64,
        "i8" => slice[0] as i8 as f64,
        "u16" => u16::from_le_bytes([slice[0], slice[1]]) as f64,
        "i16" => i16::from_le_bytes([slice[0], slice[1]]) as f64,
        "u32" => u32::from_le_bytes(slice.try_into().unwrap_or_default()) as f64,
        "i32" => i32::from_le_bytes(slice.try_into().unwrap_or_default()) as f64,
        "f32" => f32::from_le_bytes(slice.try_into().unwrap_or_default()) as f64,
        "u64" => u64::from_le_bytes(slice.try_into().unwrap_or_default()) as f64,
        "i64" => i64::from_le_bytes(slice.try_into().unwrap_or_default()) as f64,
        _ => f64::from_le_bytes(slice.try_into().unwrap_or_default()),
    };
    serde_json::Number::from_f64(number).map(Value::Number).unwrap_or(Value::Null)
}

/// `key=value` lines, then optionally `@@text` or `@@bytes` and a body.
pub fn parse(target: &Target, output: &str) -> Map<String, Value> {
    let mut sample = Map::new();
    let separator = output.lines().position(|line| matches!(line.trim_end_matches('\r'), "@@text" | "@@bytes"));
    let mut lines = output.split('\n');
    let head: Vec<&str> = match separator {
        Some(count) => lines.by_ref().take(count + 1).collect(),
        None => lines.by_ref().collect(),
    };
    for line in head.iter().take(separator.unwrap_or(usize::MAX)) {
        let Some((key, value)) = line.split_once('=').filter(|(key, _)| !key.is_empty()) else { continue };
        let value = value.trim();
        let parsed = value.parse::<f64>().ok().filter(|n| n.is_finite() && !value.is_empty()).and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::String(value.into()));
        sample.insert(key.into(), parsed);
    }
    let exists = sample.get("exists").and_then(Value::as_f64) == Some(1.0);
    sample.insert("exists".into(), Value::Bool(exists));
    if separator.is_some() {
        let body = lines.collect::<Vec<_>>().join("\n");
        match target {
            Target::Log { .. } => {
                // Counted by the probe in raw bytes: the text may have been decoded lossily.
                let next = sample.get("next").and_then(Value::as_f64).unwrap_or_else(|| sample.get("offset").and_then(Value::as_f64).unwrap_or(0.0) + body.len() as f64);
                sample.insert("nextOffset".into(), Value::from(next as u64));
                sample.insert("text".into(), Value::String(body));
            }
            Target::SharedMemory { layout, .. } => {
                let hex: String = body.chars().filter(char::is_ascii_hexdigit).collect();
                let bytes: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect();
                for field in layout {
                    sample.insert(field.field.clone(), read_field(&bytes, field));
                }
            }
            _ => {}
        }
    }
    sample
}

/// Windows reports CPU as cumulative seconds, so usage is the rate between two samples, in the
/// same units as ps: 100 means one full core.
pub fn with_cpu_rate(sample: &mut Map<String, Value>, previous: Option<&Map<String, Value>>, elapsed_seconds: f64) {
    let (Some(now), Some(before)) = (sample.get("cpuSeconds").and_then(Value::as_f64), previous.and_then(|p| p.get("cpuSeconds")).and_then(Value::as_f64)) else { return };
    if elapsed_seconds > 0.0 && previous.and_then(|p| p.get("pid")) == sample.get("pid") {
        sample.insert("cpu".into(), Value::from(((now - before) / elapsed_seconds * 100.0).max(0.0)));
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub field: String,
    pub op: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub value: Value,
    pub for_samples: u32,
}

pub fn condition(input: &Value) -> Result<Condition, String> {
    let for_samples = match &input["forSamples"] {
        Value::Null => 1,
        value => value.as_u64().filter(|n| *n <= 1000).ok_or("连续次数格式不正确。")? as u32,
    };
    let op = input["op"].as_str().unwrap_or_default();
    if op == "gone" {
        return Ok(Condition { field: "exists".into(), op: "gone".into(), value: Value::Null, for_samples });
    }
    let field = input["field"].as_str().unwrap_or_default();
    if !(1..=64).contains(&field.len()) || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("请选择要判断的字段。".into());
    }
    if op == "contains" {
        let text = input["value"].as_str().filter(|text| !text.is_empty() && text.chars().count() <= 500).ok_or("请填写要匹配的文本。")?;
        return Ok(Condition { field: field.into(), op: op.into(), value: Value::String(text.into()), for_samples });
    }
    if !matches!(op, ">" | ">=" | "<" | "<=" | "==" | "!=") || !input["value"].as_f64().is_some_and(f64::is_finite) {
        return Err("条件必须是一个比较和一个数值。".into());
    }
    Ok(Condition { field: field.into(), op: op.into(), value: input["value"].clone(), for_samples })
}

pub fn holds(condition: &Condition, sample: &Map<String, Value>) -> bool {
    if condition.op == "gone" {
        return sample.get("exists") == Some(&Value::Bool(false));
    }
    let value = sample.get(&condition.field);
    if condition.op == "contains" {
        return value.and_then(Value::as_str).zip(condition.value.as_str()).is_some_and(|(text, needle)| text.contains(needle));
    }
    let (Some(a), Some(b)) = (value.and_then(Value::as_f64), condition.value.as_f64()) else { return false };
    match condition.op.as_str() {
        ">" => a > b,
        ">=" => a >= b,
        "<" => a < b,
        "<=" => a <= b,
        "==" => a == b,
        _ => a != b,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub pid: u64,
    pub ppid: u64,
    pub cpu: f64,
    pub memory_kb: u64,
    pub command: String,
    pub cpu_ticks: Option<f64>,
}

pub fn process_list_script(platform: Platform) -> &'static str {
    if platform == Platform::Windows {
        "$ErrorActionPreference = 'SilentlyContinue'\n[Console]::OutputEncoding = [Text.Encoding]::UTF8\nGet-CimInstance Win32_Process | ForEach-Object { \"{0}`t{1}`t{2}`t{3}`t{4}\" -f $_.ProcessId, $_.ParentProcessId, ([int64]$_.KernelModeTime + [int64]$_.UserModeTime), ([int64]($_.WorkingSetSize / 1024)), $_.Name }\n"
    } else {
        "export LC_ALL=C\nps -A -o pid=,ppid=,pcpu=,rss=,comm=\n"
    }
}

pub fn parse_processes(output: &str, platform: Platform) -> Vec<Process> {
    let mut list = Vec::new();
    for line in output.lines() {
        if platform == Platform::Windows {
            let fields: Vec<&str> = line.trim_end().splitn(5, '\t').collect();
            if fields.len() < 5 || !fields[0].chars().all(|c| c.is_ascii_digit()) || fields[0].is_empty() {
                continue;
            }
            list.push(Process { pid: fields[0].parse().unwrap_or(0), ppid: fields[1].parse().unwrap_or(0), cpu: 0.0, memory_kb: fields[3].parse().unwrap_or(0), command: fields[4].chars().take(200).collect(), cpu_ticks: fields[2].parse().ok() });
        } else {
            let mut parts = line.split_whitespace();
            let (Some(pid), Some(ppid), Some(cpu), Some(rss)) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
            let command: String = parts.collect::<Vec<_>>().join(" ");
            let (Ok(pid), Ok(ppid), Ok(cpu), Ok(rss)) = (pid.parse(), ppid.parse(), cpu.parse::<f64>(), rss.parse()) else { continue };
            if command.is_empty() {
                continue;
            }
            list.push(Process { pid, ppid, cpu, memory_kb: rss, command: command.chars().take(200).collect(), cpu_ticks: None });
        }
        if list.len() >= MAX_PROCESSES {
            break;
        }
    }
    list
}

/// CPU from tick counters, between two lists; the first list has no rate to show.
pub fn with_process_rates(list: &mut [Process], previous: &[Process], elapsed_seconds: f64) {
    if elapsed_seconds <= 0.0 {
        return;
    }
    let before: std::collections::HashMap<u64, f64> = previous.iter().filter_map(|p| Some((p.pid, p.cpu_ticks?))).collect();
    for process in list {
        if let (Some(now), Some(then)) = (process.cpu_ticks, before.get(&process.pid)) {
            process.cpu = ((now - then) / TICKS_PER_SECOND / elapsed_seconds * 100.0).max(0.0);
        }
    }
}

/// Runs a probe on this machine. The PowerShell script travels encoded, so nothing in it
/// passes through a command-line parser.
pub fn run_local(script: &str) -> Result<String, String> {
    let mut command = if cfg!(windows) {
        let encoded: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut command = crate::remote::manager::command("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &base64(&encoded)]);
        command
    } else {
        let mut command = crate::remote::manager::command("sh");
        command.args(["-c", script]);
        command
    };
    run_with_timeout(&mut command, std::time::Duration::from_secs(15)).ok_or_else(|| "本机采样失败。".into())
}

pub fn run_with_timeout(command: &mut std::process::Command, timeout: std::time::Duration) -> Option<String> {
    use std::io::Read;
    let mut child = command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.by_ref().take(2 << 20).read_to_end(&mut output);
        output
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                break None;
            }
        }
    };
    let output = reader.join().ok()?;
    status.filter(|status| status.success()).map(|_| String::from_utf8_lossy(&output).into_owned())
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn targets_are_validated_before_they_reach_a_script() {
        assert!(validate(&json!({ "kind": "process", "name": "a;rm" }), Platform::Linux).is_err());
        assert!(validate(&json!({ "kind": "log", "path": "/var/log/../etc" }), Platform::Linux).is_err());
        assert!(validate(&json!({ "kind": "log", "path": "/var/log/it's" }), Platform::Linux).is_err());
        assert!(validate(&json!({ "kind": "log", "path": "D:\\日志\\服务 1\\app.log" }), Platform::Windows).is_ok());
        assert!(validate(&json!({ "kind": "log", "path": "C:\\a\u{2019}b" }), Platform::Windows).is_err());
        assert!(validate(&json!({ "kind": "sharedMemory", "name": "x", "access": "posix-shm", "layout": [{ "field": "a", "offset": 0, "type": "u8" }] }), Platform::Windows).is_err());
    }

    #[test]
    fn every_posix_probe_avoids_single_quotes() {
        let targets = [json!({ "kind": "process", "name": "nginx" }), json!({ "kind": "thread", "pid": 1, "tid": 2 }), json!({ "kind": "log", "path": "/var/log/a b.log" })];
        for target in targets {
            let target = validate(&target, Platform::Linux).unwrap();
            assert!(!script(&target, &State { offset: Some(4), mark: Some("x'; rm".into()) }, Platform::Linux).unwrap().contains('\''));
        }
    }

    #[test]
    fn samples_parse_keys_logs_and_shared_memory() {
        let log = Target::Log { path: "/a".into() };
        let sample = parse(&log, "exists=1\nsize=10\noffset=4\nnext=10\nmark=1-2\n@@text\nERROR x\n");
        assert_eq!(sample["exists"], json!(true));
        assert_eq!(sample["nextOffset"], json!(10));
        assert_eq!(sample["text"], json!("ERROR x\n"));
        let region = validate(&json!({ "kind": "sharedMemory", "name": "s", "access": "posix-shm", "layout": [{ "field": "count", "offset": 0, "type": "u16" }] }), Platform::Linux).unwrap();
        assert_eq!(parse(&region, "exists=1\n@@bytes\n2a00\n")["count"], json!(42.0));
    }

    #[test]
    fn conditions_cover_numbers_text_and_disappearance() {
        let gone = condition(&json!({ "op": "gone" })).unwrap();
        let mut sample = Map::new();
        sample.insert("exists".into(), json!(false));
        assert!(holds(&gone, &sample));
        let busy = condition(&json!({ "field": "cpu", "op": ">", "value": 50 })).unwrap();
        sample.insert("cpu".into(), json!(80.0));
        assert!(holds(&busy, &sample));
        assert!(condition(&json!({ "field": "cpu", "op": "~", "value": 1 })).is_err());
    }

    #[test]
    fn process_lists_parse_ps_output_with_spaces_in_names() {
        let list = parse_processes("    1     0  0.0  1234 init\n  42     1 12.5  999 Web Content\nnoise\n", Platform::Linux);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].command, "Web Content");
        assert_eq!(base64(b"hi!"), "aGkh");
        assert_eq!(base64(b"h"), "aA==");
    }
}
