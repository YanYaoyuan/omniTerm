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
