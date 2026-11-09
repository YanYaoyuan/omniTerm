// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Real OpenSSH/SFTP regression tests. `scripts/test-sftp.py` provides an isolated,
//! rate-limited loopback server; no personal SSH files or remote hosts are used.

use super::*;

struct Fixture {
    remote: Arc<Remote>,
    local: PathBuf,
    server: PathBuf,
    disconnect: PathBuf,
}

fn until(description: &str, seconds: u64, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !check() {
        assert!(Instant::now() < deadline, "timed out: {description}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Fixture {
    fn new() -> Self {
        let config = std::env::var("OMNITERM_SFTP_E2E").expect("run with scripts/test-sftp.py");
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
        let root = Path::new(config["root"].as_str().unwrap()).join(uuid::Uuid::new_v4().simple().to_string());
        let local = root.join("local");
        let server = root.join("server");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        let disconnect = root.join("disconnect");
        let manager = Manager::new(|| {});
        manager.attach("test", Companion {
            executable: "ssh".into(), destination: config["destination"].as_str().unwrap().into(), control_path: None,
            args: vec!["-F".into(), "/dev/null".into(), "-p".into(), config["port"].to_string(), "-i".into(), config["key"].as_str().unwrap().into(),
                "-o".into(), "IdentityAgent=none".into(), "-o".into(), "IdentitiesOnly=yes".into(),
                "-o".into(), format!("UserKnownHostsFile={}", config["known_hosts"].as_str().unwrap()), "-o".into(), "StrictHostKeyChecking=yes".into(),
                "-o".into(), format!("SetEnv=OMNITERM_SFTP_CONTROL={}", disconnect.display())],
        });
        let remote = manager.get("test").unwrap();
        until("SFTP ready", 10, || {
            let snapshot = remote.snapshot();
            assert!(!matches!(snapshot.phase, Phase::Unavailable { .. }), "SFTP connection failed: {:?}", snapshot.phase);
            snapshot.phase == Phase::Ready
        });
        Self { remote, local, server, disconnect }
    }

    fn wait(&self, count: usize) -> Vec<Transfer> {
        until("transfers finish", 15, || {
            let snapshot = self.remote.snapshot();
            snapshot.transfers.len() == count && snapshot.transfers.iter().all(Transfer::finished)
        });
        self.remote.snapshot().transfers
    }

    fn uploading(&self, source: &Path) -> u64 {
        self.remote.upload(vec![source.to_path_buf()], self.server.to_string_lossy().into_owned());
        self.started()
    }

    fn started(&self) -> u64 {
        until("transfer has received bytes", 8, || self.remote.snapshot().transfers.last().is_some_and(|transfer| transfer.bytes >= 128 * 1024 && !transfer.finished()));
        self.remote.snapshot().transfers.last().unwrap().id
    }

    fn drop_connection(&self) {
        std::fs::write(&self.disconnect, b"drop this fixture's SSH connection").unwrap();
        until("connection loss reported", 8, || matches!(self.remote.snapshot().phase, Phase::Unavailable { reason: Reason::Lost, .. }));
    }

    fn reconnect(&self) {
        std::fs::remove_file(&self.disconnect).unwrap();
        self.remote.retry();
        until("SFTP reconnected", 10, || self.remote.snapshot().phase == Phase::Ready);
    }

    fn no_partials(&self) {
        fn scan(directory: &Path) {
            for entry in std::fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                assert!(!name.ends_with(".part") && !name.ends_with(".omniterm-part"), "leftover partial: {}", entry.path().display());
                if entry.file_type().unwrap().is_dir() { scan(&entry.path()); }
            }
        }
        scan(&self.local);
        scan(&self.server);
    }

    fn large_file(path: &Path) {
        // Sparse input keeps the fixture small, with SSH compression left off.
        std::fs::File::create(path).unwrap().set_len(4 * 1024 * 1024).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The server watches a fixture-specific marker, so cleanup works even if the
        // failure being tested is a client-side shutdown deadlock.
        let _ = std::fs::write(&self.disconnect, b"cleanup");
        if let Some(worker) = self.remote.close() { let _ = worker.join(); }
    }
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_batch_and_nested_round_trip() {
    let fixture = Fixture::new();
    let tree = fixture.local.join("目录 with spaces #%");
    std::fs::create_dir_all(tree.join("nested/empty-directory")).unwrap();
    std::fs::write(tree.join("nested/报告 #1%.txt"), "字节完整性\n".repeat(2000)).unwrap();
    std::fs::write(tree.join("empty.bin"), b"").unwrap();
    std::os::unix::fs::symlink("nested/报告 #1%.txt", tree.join("skip-link")).unwrap();
    let names = ["one.bin", "two with spaces.bin", "中文 #%.bin", "zero.bin"];
    let mut sources = vec![tree.clone()];
    for (index, name) in names.iter().enumerate() {
        let path = fixture.local.join(name);
        std::fs::write(&path, vec![index as u8; index * 17011]).unwrap(); sources.push(path);
    }
    assert!(fixture.remote.upload_conflicts(&sources, fixture.server.to_str().unwrap()).unwrap().is_empty());
    fixture.remote.upload(sources.clone(), fixture.server.to_string_lossy().into_owned());
    let transfers = fixture.wait(5);
    assert!(transfers.iter().all(|transfer| transfer.status == TransferStatus::Done && transfer.bytes == transfer.total), "{transfers:?}");
    assert_eq!(transfers.iter().map(|transfer| transfer.skipped).sum::<usize>(), 1);
    assert_eq!(fixture.remote.upload_conflicts(&sources, fixture.server.to_str().unwrap()).unwrap().len(), 5);
    let downloads = fixture.local.join("downloads"); std::fs::create_dir(&downloads).unwrap();
    let paths: Vec<String> = sources.iter().map(|path| fixture.server.join(path.file_name().unwrap()).to_string_lossy().into_owned()).collect();
    fixture.remote.download(paths, downloads.clone()).unwrap();
    assert!(fixture.wait(10).iter().all(|transfer| transfer.status == TransferStatus::Done));
    for name in names { assert_eq!(std::fs::read(fixture.local.join(name)).unwrap(), std::fs::read(downloads.join(name)).unwrap()); }
    assert_eq!(std::fs::read(tree.join("nested/报告 #1%.txt")).unwrap(), std::fs::read(downloads.join(tree.file_name().unwrap()).join("nested/报告 #1%.txt")).unwrap());
    assert!(downloads.join(tree.file_name().unwrap()).join("nested/empty-directory").is_dir());
    assert!(!fixture.server.join(tree.file_name().unwrap()).join("skip-link").exists());
    fixture.no_partials();
    println!("sftp-e2e batch, nested/empty directories, zero-byte files, Unicode/special names, skipped symlinks: byte-identical round trip, no partials");
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_cancel_running_upload_and_download() {
    let fixture = Fixture::new();
    let source = fixture.local.join("cancel.bin"); Fixture::large_file(&source);
    let destination = fixture.server.join("cancel.bin"); std::fs::write(&destination, b"keep original remote file").unwrap();
    let id = fixture.uploading(&source); fixture.remote.cancel(id);
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Canceled);
    assert_eq!(std::fs::read(&destination).unwrap(), b"keep original remote file");
    fixture.remote.clear_finished();
    Fixture::large_file(&destination); std::fs::write(&source, b"keep original local file").unwrap();
    fixture.remote.download(vec![destination.to_string_lossy().into_owned()], fixture.local.clone()).unwrap();
    let id = fixture.started(); fixture.remote.cancel(id);
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Canceled);
    assert_eq!(std::fs::read(&source).unwrap(), b"keep original local file");
    fixture.no_partials();
    println!("sftp-e2e running upload/download canceled: originals intact, partials removed");
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_cancel_queued_is_immediate() {
    let fixture = Fixture::new();
    let sources: Vec<PathBuf> = ["active1.bin", "active2.bin", "queued.bin"].iter().map(|name| { let path = fixture.local.join(name); Fixture::large_file(&path); path }).collect();
    fixture.remote.upload(sources, fixture.server.to_string_lossy().into_owned());
    let snapshot = fixture.remote.snapshot();
    assert_eq!(snapshot.transfers[2].status, TransferStatus::Queued);
    fixture.remote.cancel(snapshot.transfers[2].id);
    assert_eq!(fixture.remote.snapshot().transfers[2].status, TransferStatus::Canceled, "queued cancellation must not wait for busy transfers");
    let drag = fixture.server.join("queued-drag.bin"); Fixture::large_file(&drag);
    fixture.remote.prepare_drag(drag.to_string_lossy().into_owned()).unwrap();
    let drag_id = fixture.remote.snapshot().transfers.last().unwrap().id;
    fixture.remote.cancel(drag_id);
    assert!(!fixture.remote.snapshot().drags.contains_key(drag.to_str().unwrap()), "canceled drag must not stay in Preparing");
    for transfer in snapshot.transfers.iter().take(2) { fixture.remote.cancel(transfer.id); }
    assert!(fixture.wait(4).iter().all(|transfer| transfer.status == TransferStatus::Canceled));
    assert!(!fixture.server.join("queued.bin").exists()); fixture.no_partials();
    println!("sftp-e2e queued cancellation immediate: no remote file created");
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_disconnect_upload_and_retry() {
    let fixture = Fixture::new();
    let source = fixture.local.join("interrupted.bin"); Fixture::large_file(&source);
    let destination = fixture.server.join("interrupted.bin"); std::fs::write(&destination, b"remote original").unwrap();
    fixture.uploading(&source); fixture.drop_connection();
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Failed);
    assert_eq!(std::fs::read(&destination).unwrap(), b"remote original");
    fixture.reconnect(); fixture.no_partials();
    fixture.remote.clear_finished(); std::fs::write(&source, b"retry after reconnect").unwrap();
    fixture.remote.upload(vec![source], fixture.server.to_string_lossy().into_owned());
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Done);
    assert_eq!(std::fs::read(destination).unwrap(), b"retry after reconnect"); fixture.no_partials();
    println!("sftp-e2e mid-upload disconnect: original intact, partial cleaned on reconnect, subsequent upload succeeds");
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_disconnect_download_and_retry() {
    let fixture = Fixture::new();
    let source = fixture.server.join("interrupted.bin"); Fixture::large_file(&source);
    let destination = fixture.local.join("interrupted.bin"); std::fs::write(&destination, b"local original").unwrap();
    fixture.remote.download(vec![source.to_string_lossy().into_owned()], fixture.local.clone()).unwrap();
    fixture.started(); fixture.drop_connection();
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Failed);
    assert_eq!(std::fs::read(&destination).unwrap(), b"local original"); fixture.no_partials();
    fixture.reconnect(); fixture.remote.clear_finished(); std::fs::write(&source, b"download after reconnect").unwrap();
    fixture.remote.download(vec![source.to_string_lossy().into_owned()], fixture.local.clone()).unwrap();
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Done);
    assert_eq!(std::fs::read(destination).unwrap(), b"download after reconnect"); fixture.no_partials();
    println!("sftp-e2e mid-download disconnect: original intact, partial removed, subsequent download succeeds");
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_close_does_not_wait_for_ssh_exit() {
    let fixture = Fixture::new();
    let source = fixture.local.join("closing.bin"); Fixture::large_file(&source); fixture.uploading(&source);
    let remote = fixture.remote.clone();
    let (done, waiting) = std::sync::mpsc::channel();
    std::thread::spawn(move || { let worker = remote.close(); let _ = done.send(worker); });
    let result = waiting.recv_timeout(Duration::from_secs(2));
    let closed = result.is_ok();
    // Unblock the old implementation before asserting, so a regression never wedges CI.
    if !closed { std::fs::write(&fixture.disconnect, b"unblock close").unwrap(); let _ = waiting.recv_timeout(Duration::from_secs(5)); }
    assert!(closed, "closing an SSH tab blocked on the SFTP child-process waiter");
    if let Some(worker) = result.unwrap() { worker.join().unwrap(); }
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Canceled);
    fixture.no_partials();
    fixture.remote.connect();
    assert!(lock(&fixture.remote.state).sftp_child.is_none(), "a late retry must not reopen a closed tab's connection");
    println!("sftp-e2e SSH tab close returned immediately; canceled worker cleaned its temporary file before shutdown");
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_failed_items_do_not_break_the_queue() {
    let fixture = Fixture::new();
    let source = fixture.local.join("existing-directory"); std::fs::write(&source, b"cannot replace a directory").unwrap();
    let target = fixture.server.join("existing-directory"); std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"keep this directory").unwrap();
    assert_eq!(fixture.remote.upload_conflicts(std::slice::from_ref(&source), fixture.server.to_str().unwrap()).unwrap(), ["existing-directory"]);
    fixture.remote.upload(vec![source], fixture.server.to_string_lossy().into_owned());
    let transfers = fixture.wait(1);
    assert_eq!(transfers[0].status, TransferStatus::Failed); assert!(!transfers[0].error.is_empty());
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"keep this directory"); fixture.no_partials();
    fixture.remote.download(vec![fixture.server.join("missing-file.bin").to_string_lossy().into_owned()], fixture.local.clone()).unwrap();
    assert_eq!(fixture.wait(2)[1].status, TransferStatus::Failed);
    let good = fixture.local.join("next.bin"); std::fs::write(&good, b"queue still works").unwrap();
    fixture.remote.upload(vec![good], fixture.server.to_string_lossy().into_owned());
    assert_eq!(fixture.wait(3)[2].status, TransferStatus::Done);
    assert_eq!(std::fs::read(fixture.server.join("next.bin")).unwrap(), b"queue still works"); fixture.no_partials();
    println!("sftp-e2e failed upload/download keep originals, report errors, remove partials, and allow subsequent queued work");
}

#[test]
#[ignore = "needs isolated loopback sshd; run scripts/test-sftp.py"]
fn sftp_end_to_end_shutdown_finishes_detached_tab_cleanup() {
    let fixture = Fixture::new();
    let source = fixture.local.join("shutdown.bin"); Fixture::large_file(&source); fixture.uploading(&source);
    let manager = Manager::new(|| {});
    lock(&manager.remotes).insert("test".into(), fixture.remote.clone());
    manager.detach("test");
    manager.shutdown();
    assert_eq!(fixture.wait(1)[0].status, TransferStatus::Canceled); fixture.no_partials();
    println!("sftp-e2e application shutdown joined cleanup of an already detached SSH tab");
}


/// A local byte-stream peer, never an SSH server. It exercises the production SFTP
/// reader/manager and child lifecycle without network, keys, an askpass GUI or global env.
struct WireFixture {
    directory: PathBuf,
    executable: String,
    manager: Manager,
}

impl WireFixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let directory = private_directory(&std::env::temp_dir(), "omniterm-file-wire-").unwrap();
        let events = directory.join("events.jsonl");
        let script = directory.join("ssh-fixture");
        let python = crate::core::paths::find_executable("python3").expect("python3 for local wire fixture");
        let program = format!("#!{}\nROOT = {:?}\nEVENTS = {:?}\n", python.display(), directory.to_string_lossy(), events.to_string_lossy()) + r#"
import json, os, pathlib, struct, sys, time
root = pathlib.Path(ROOT)
with open(EVENTS, 'a') as log:
    log.write(json.dumps({'args':sys.argv[1:], 'auth':os.environ.get('OMNITERM_QIZHI_AUTH_STATE'), 'pid':os.getpid()}) + '\n')
if '-O' in sys.argv:
    (root / 'master-exit').write_text('master exit requested')
    sys.exit(0)
if any(arg.startswith('sh -c ') for arg in sys.argv):
    (root / 'metrics-exec').write_text('metrics requested')
    sys.exit(0)
if (root / 'spawn-descendant').exists():
    import subprocess
    code = "import pathlib,time; p=pathlib.Path(" + repr(str(root / 'heartbeat')) + "); " + "exec(\"while True:\\n p.open('ab').write(b'x')\\n time.sleep(0.02)\")"
    subprocess.Popen([sys.executable, '-c', code], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while (root / 'hold').exists(): time.sleep(0.01)
if (root / 'fail').exists(): sys.exit(23)
def u32(n): return struct.pack('>I', n)
def text(s):
    s = s.encode('utf-8') if isinstance(s, str) else s
    return u32(len(s)) + s
def readn(n):
    data = b''
    while len(data) < n:
        part = sys.stdin.buffer.read(n - len(data))
        if not part: raise EOFError()
        data += part
    return data
def send(packet):
    sys.stdout.buffer.write(u32(len(packet)) + packet)
    sys.stdout.buffer.flush()
def string(packet, offset=5):
    length = struct.unpack('>I', packet[offset:offset+4])[0]
    return packet[offset+4:offset+4+length]
try:
    packet = readn(struct.unpack('>I', readn(4))[0])
    assert packet[0] == 1
    send(bytes([2]) + u32(3))
    listed = False
    while True:
        packet = readn(struct.unpack('>I', readn(4))[0])
        kind, ident = packet[0], packet[1:5]
        if kind == 16:
            path = string(packet)
            with open(root / 'paths', 'ab') as log: log.write(path + b'\n')
            # A stale reply with another request id must not consume this request.
            send(bytes([101]) + u32(0xFFFFFFFE) + u32(0))
            send(bytes([104]) + ident + u32(1) + text(path) + text(b'') + u32(0))
        elif kind == 11:
            listed = False
            send(bytes([102]) + ident + text('fixture-handle'))
        elif kind == 12 and not listed:
            listed = True
            send(bytes([104]) + ident + u32(1) + text('fixture.txt') + text('fixture.txt') + u32(4) + u32(0o100644))
        else:
            send(bytes([101]) + ident + u32(1 if kind == 12 else 0) + text('') + text(''))
except (EOFError, BrokenPipeError):
    pass
"#;
        std::fs::write(&script, program).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { directory, executable: script.to_string_lossy().into_owned(), manager: Manager::new(|| {}) }
    }

    fn connection(&self, directory: &str) -> qizhi::Connection {
        let file = self.directory.join(format!("import-{}.json", uuid::Uuid::new_v4().simple()));
        let document = serde_json::json!({"parameters": {
            "app": "winscp", "Host": "fixture.invalid", "User": "fixture/192.0.2.10/root",
            "Port": "60022", "Path": directory, "pwd": "synthetic-single-use-secret", "st": "Fixture"
        }});
        let mut options = std::fs::OpenOptions::new();
        use std::os::unix::fs::OpenOptionsExt;
        let mut file_handle = options.write(true).create_new(true).mode(0o600).open(&file).unwrap();
        use std::io::Write;
        file_handle.write_all(&serde_json::to_vec(&document).unwrap()).unwrap();
        let result = qizhi::load(&file).unwrap();
        std::fs::remove_file(file).unwrap();
        result
    }

    fn attach(&self, id: &str, directory: &str) -> Arc<Remote> {
        self.manager.install(id, Target::Independent { connection: Mutex::new(self.connection(directory)), executable: self.executable.clone() });
        self.manager.get(id).unwrap()
    }

    fn events(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.directory.join("events.jsonl")).unwrap_or_default().lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
    }

    fn ready(&self, remote: &Remote) {
        until("wire SFTP ready", 5, || {
            let phase = remote.snapshot().phase;
            assert!(!matches!(phase, Phase::Unavailable { .. }), "wire fixture failed: {phase:?}");
            phase == Phase::Ready
        });
    }
}

impl Drop for WireFixture {
    fn drop(&mut self) {
        self.manager.shutdown();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn independent_file_connections_keep_panes_separate_and_never_execute_shell_commands() {
    let fixture = WireFixture::new();
    let a = fixture.attach("pane-a", "/asset/a");
    let b = fixture.attach("pane-b", "/asset/b");
    fixture.ready(&a); fixture.ready(&b);
    assert!(a.is_independent() && b.is_independent());
    assert_eq!(a.snapshot().home.as_deref(), Some("/asset/a"));
    assert_eq!(b.snapshot().home.as_deref(), Some("/asset/b"));
    a.list("/asset/a".into()); b.list("/asset/b".into());
    until("each pane's directory listing", 5, || !a.snapshot().listing.loading && !b.snapshot().listing.loading);
    assert_eq!(a.snapshot().listing.path.as_deref(), Some("/asset/a"));
    assert_eq!(b.snapshot().listing.path.as_deref(), Some("/asset/b"));
    assert_eq!(a.snapshot().listing.entries[0].name, "fixture.txt");
    assert!(a.exec("id", Duration::from_secs(1)).unwrap_err().contains("仅支持 SFTP"));
    a.watch_metrics(true); b.watch_metrics(true);
    assert!(!lock(&a.state).metrics_wanted && lock(&a.state).metrics_child.is_none());
    assert!(!lock(&b.state).metrics_wanted && lock(&b.state).metrics_child.is_none());
    let events = fixture.events();
    assert_eq!(events.len(), 2);
    for event in &events {
        let args: Vec<&str> = event["args"].as_array().unwrap().iter().map(|value| value.as_str().unwrap()).collect();
        assert_eq!(&args[args.len()-3..], ["-s", "fixture.invalid", "sftp"]);
        assert!(args.contains(&"ControlPath=none") && args.contains(&"ControlMaster=no"));
        assert!(!event["args"].to_string().contains("synthetic-single-use-secret"));
    }
    fixture.manager.detach("pane-a");
    assert!(fixture.manager.get("pane-a").is_none());
    assert_eq!(b.snapshot().phase, Phase::Ready);
    fixture.manager.shutdown();
    for event in events { assert!(!Path::new(event["auth"].as_str().unwrap()).exists(), "auth state must be removed on close"); }
    assert!(!fixture.directory.join("master-exit").exists());
    assert!(!fixture.directory.join("metrics-exec").exists());
}

#[test]
fn replacing_shared_files_preserves_terminal_master_and_rejects_stale_results() {
    let fixture = WireFixture::new();
    let socket = fixture.directory.join("terminal-socket");
    std::fs::write(&socket, b"fixture socket marker").unwrap();
    let shared = Companion { executable: fixture.executable.clone(), destination: "fixture.invalid".into(), control_path: Some(socket.to_string_lossy().into_owned()), args: vec![] };
    fixture.manager.attach("pane", shared);
    let previous = fixture.manager.get("pane").unwrap();
    fixture.ready(&previous);
    let args = previous.channel_args();
    assert!(args.iter().any(|arg| arg == "BatchMode=yes"));
    assert!(args.iter().any(|arg| arg == "ProxyCommand=false"));
    let independent = fixture.attach("pane", "/replacement");
    fixture.ready(&independent);
    assert!(previous.closed.load(Ordering::Acquire));
    previous.publish_attempt(0, |state| state.phase = Phase::Unavailable { reason: Reason::Failed, detail: "stale result".into() });
    assert_eq!(fixture.manager.get("pane").unwrap().snapshot().home.as_deref(), Some("/replacement"));
    fixture.manager.detach_file_only("pane");
    assert!(!fixture.directory.join("master-exit").exists(), "replacing file channels must not exit terminal master");
    fixture.manager.detach("pane");
    fixture.manager.shutdown();
    assert_eq!(fixture.events().iter().filter(|event| event["args"].as_array().unwrap().iter().any(|arg| arg == "-O")).count(), 1, "terminal detach must exit the original master exactly once");
}

#[test]
fn independent_retry_is_explicit_serialized_and_does_not_resend_imported_secret() {
    let fixture = WireFixture::new();
    std::fs::write(fixture.directory.join("fail"), b"fail initial attempt").unwrap();
    let remote = fixture.attach("pane", "/retry");
    until("first failure", 5, || matches!(remote.snapshot().phase, Phase::Unavailable { .. }));
    assert_eq!(fixture.events().len(), 1);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(fixture.events().len(), 1, "no automatic retry");
    std::fs::remove_file(fixture.directory.join("fail")).unwrap();
    std::fs::write(fixture.directory.join("hold"), b"hold retry during authentication").unwrap();
    std::thread::scope(|scope| { for _ in 0..16 { let remote = remote.clone(); scope.spawn(move || remote.retry()); } });
    until("one retry starts", 5, || fixture.events().len() == 2);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(fixture.events().len(), 2, "concurrent retries must share one attempt");
    let events = fixture.events();
    let auth = Path::new(events[1]["auth"].as_str().unwrap());
    assert!(!auth.join("credential").exists(), "retry must prompt instead of replaying the import credential");
    std::fs::remove_file(fixture.directory.join("hold")).unwrap();
    fixture.ready(&remote);
    let before = remote.snapshot().phase;
    remote.publish_attempt(1, |state| state.phase = Phase::Unavailable { reason: Reason::Failed, detail: "late old error".into() });
    assert_eq!(remote.snapshot().phase, before, "old attempts cannot overwrite retry success");
}

#[test]
fn independent_authentication_can_outlast_shared_handshake_and_cancel_cleans_children() {
    let fixture = WireFixture::new();
    std::fs::write(fixture.directory.join("hold"), b"simulate user entering OTP").unwrap();
    std::fs::write(fixture.directory.join("spawn-descendant"), b"simulate askpass child").unwrap();
    let remote = fixture.attach("pane", "/otp");
    until("independent child starts", 5, || !fixture.events().is_empty());
    std::thread::sleep(HANDSHAKE + Duration::from_millis(250));
    assert_eq!(remote.snapshot().phase, Phase::Connecting, "OTP entry must not hit the shared 20 second watchdog");
    let child = lock(&remote.state).sftp_child.clone().unwrap();
    let events = fixture.events();
    let auth = Path::new(events[0]["auth"].as_str().unwrap());
    assert!(auth.exists());
    fixture.manager.detach_file_only("pane");
    fixture.manager.shutdown();
    assert!(lock(&child).try_wait().unwrap().is_some());
    assert!(!auth.exists());
    let heartbeat = fixture.directory.join("heartbeat");
    let size = std::fs::metadata(&heartbeat).expect("descendant ran").len();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(std::fs::metadata(&heartbeat).unwrap().len(), size, "cancel must also stop the ssh child's descendants");
    remote.retry();
    assert_eq!(fixture.events().len(), 1, "closed connections cannot restart");
}

#[test]
fn ordinary_shared_channels_still_sample_and_close_their_master() {
    let fixture = WireFixture::new();
    let socket = fixture.directory.join("ordinary-socket");
    std::fs::write(&socket, b"fixture socket marker").unwrap();
    fixture.manager.attach("ordinary", Companion { executable: fixture.executable.clone(), destination: "fixture.invalid".into(), control_path: Some(socket.to_string_lossy().into_owned()), args: vec![] });
    let remote = fixture.manager.get("ordinary").unwrap();
    fixture.ready(&remote);
    assert!(!remote.is_independent());
    assert!(remote.exec("id", Duration::from_secs(2)).is_ok());
    remote.watch_metrics(true);
    until("ordinary metrics starts", 5, || fixture.events().iter().filter(|event| event["args"].to_string().contains("sh -c ")).count() >= 2);
    fixture.manager.detach("ordinary");
    fixture.manager.shutdown();
    assert!(fixture.directory.join("master-exit").exists(), "ordinary SSH tab shutdown must still exit its shared master");
}

#[test]
fn shutdown_closes_preserved_masters_once_after_repeated_file_replacements() {
    let fixture = WireFixture::new();
    for number in [1, 1, 2] {
        let socket = fixture.directory.join(format!("preserved-{number}"));
        std::fs::write(&socket, b"fixture socket marker").unwrap();
        fixture.manager.attach("pane", Companion { executable: fixture.executable.clone(), destination: "fixture.invalid".into(), control_path: Some(socket.to_string_lossy().into_owned()), args: vec![] });
        fixture.ready(&fixture.manager.get("pane").unwrap());
        let independent = fixture.attach("pane", "/replacement");
        fixture.ready(&independent);
        fixture.manager.detach_file_only("pane");
        assert!(!fixture.directory.join("master-exit").exists());
    }
    fixture.manager.shutdown();
    let events = fixture.events();
    let exits: Vec<_> = events.iter().filter(|event| event["args"].as_array().unwrap().iter().any(|arg| arg == "-O")).collect();
    assert_eq!(exits.len(), 2, "app shutdown must retain all original masters and deduplicate a repeated socket");
    for number in [1, 2] {
        let option = format!("ControlPath={}", fixture.directory.join(format!("preserved-{number}")).display());
        assert_eq!(exits.iter().filter(|event| event["args"].as_array().unwrap().iter().any(|arg| arg == &option)).count(), 1);
    }
    fixture.manager.shutdown();
    assert_eq!(fixture.events().len(), events.len(), "repeated shutdown cannot exit a master again");
}
