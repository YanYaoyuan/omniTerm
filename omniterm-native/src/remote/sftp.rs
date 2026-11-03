// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! A small SFTP version 3 client over any byte stream pair. It runs over `ssh -s sftp`, so
//! OpenSSH keeps handling keys, host verification and connection sharing, while file names
//! never pass through a shell or `ls` output. Requests are pipelined: transfers keep up to
//! `WINDOW` chunks in flight, which is what makes them fast on high-latency links.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const INIT: u8 = 1;
const VERSION: u8 = 2;
const OPEN: u8 = 3;
const CLOSE: u8 = 4;
const READ: u8 = 5;
const WRITE: u8 = 6;
const LSTAT: u8 = 7;
const OPENDIR: u8 = 11;
const READDIR: u8 = 12;
const REMOVE: u8 = 13;
const MKDIR: u8 = 14;
const RMDIR: u8 = 15;
const REALPATH: u8 = 16;
const STAT: u8 = 17;
const RENAME: u8 = 18;
const EXTENDED: u8 = 200;
const STATUS: u8 = 101;
const HANDLE: u8 = 102;
const DATA: u8 = 103;
const NAME: u8 = 104;
const ATTRS: u8 = 105;
const EXTENDED_REPLY: u8 = 201;

const STATUS_OK: u32 = 0;
const STATUS_EOF: u32 = 1;
pub const NO_SUCH_FILE: u32 = 2;

const ATTR_SIZE: u32 = 1;
const ATTR_UIDGID: u32 = 2;
const ATTR_PERMISSIONS: u32 = 4;
const ATTR_TIMES: u32 = 8;
const ATTR_EXTENDED: u32 = 0x8000_0000;
const OPEN_READ: u32 = 1;
const OPEN_WRITE: u32 = 2;
const OPEN_CREATE: u32 = 8;
const OPEN_TRUNCATE: u32 = 0x10;

const MAX_PACKET: usize = 1 << 20;
const MAX_ENTRIES: usize = 100_000;
/// 32 KiB is the largest payload every server must accept.
pub const CHUNK: u32 = 32 * 1024;
const WINDOW: usize = 16;
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

pub const S_IFMT: u32 = 0o170000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Attributes {
    pub size: Option<u64>,
    pub mode: Option<u32>,
    pub mtime: Option<u32>,
}

impl Attributes {
    fn kind(&self) -> u32 {
        self.mode.unwrap_or(0) & S_IFMT
    }
    pub fn directory(&self) -> bool {
        self.kind() == S_IFDIR
    }
    pub fn file(&self) -> bool {
        self.kind() == S_IFREG
    }
    pub fn symlink(&self) -> bool {
        self.kind() == S_IFLNK
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub attributes: Attributes,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Status(u32, String),
    Protocol(String),
    Closed,
    Canceled,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Status(code, message) if message.is_empty() => write!(formatter, "SFTP 请求失败（状态 {code}）。"),
            Error::Status(_, message) => formatter.write_str(message),
            Error::Protocol(message) => formatter.write_str(message),
            Error::Closed => formatter.write_str("SFTP 连接已关闭。"),
            Error::Canceled => formatter.write_str("传输已取消。"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

enum Reply {
    Status(u32, String),
    Handle(Vec<u8>),
    Data(Vec<u8>),
    Names(Vec<Entry>),
    Attrs(Attributes),
    Extended,
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(size).filter(|end| *end <= self.bytes.len()).ok_or_else(|| Error::Protocol("SFTP 服务器发送了截断的数据包。".into()))?;
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("four bytes")))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }
    fn bytes(&mut self) -> Result<&'a [u8]> {
        let size = self.u32()? as usize;
        self.take(size)
    }
    fn text(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
    fn attributes(&mut self) -> Result<Attributes> {
        let flags = self.u32()?;
        let mut attributes = Attributes::default();
        if flags & ATTR_SIZE != 0 {
            attributes.size = Some(self.u64()?);
        }
        if flags & ATTR_UIDGID != 0 {
            self.u32()?;
            self.u32()?;
        }
        if flags & ATTR_PERMISSIONS != 0 {
            attributes.mode = Some(self.u32()?);
        }
        if flags & ATTR_TIMES != 0 {
            self.u32()?;
            attributes.mtime = Some(self.u32()?);
        }
        if flags & ATTR_EXTENDED != 0 {
            for _ in 0..self.u32()? {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(attributes)
    }
    fn done(&self) -> bool {
        self.offset >= self.bytes.len()
    }
}

/// Builds one request body.
struct Packet(Vec<u8>);

impl Packet {
    fn new(kind: u8, id: u32) -> Self {
        let mut bytes = vec![0; 4];
        bytes.push(kind);
        bytes.extend_from_slice(&id.to_be_bytes());
        Self(bytes)
    }
    fn u32(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }
    fn u64(mut self, value: u64) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }
    fn bytes(mut self, value: &[u8]) -> Self {
        self.0.extend_from_slice(&(value.len() as u32).to_be_bytes());
        self.0.extend_from_slice(value);
        self
    }
    fn text(self, value: &str) -> Self {
        self.bytes(value.as_bytes())
    }
    fn mode(self, mode: Option<u32>) -> Self {
        match mode {
            Some(mode) => self.u32(ATTR_PERMISSIONS).u32(mode),
            None => self.u32(0),
        }
    }
    fn finish(mut self) -> Vec<u8> {
        let length = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&length.to_be_bytes());
        self.0
    }
}

type Pending = Arc<Mutex<HashMap<u32, Sender<Result<Reply>>>>>;

pub struct Client {
    input: Mutex<Box<dyn Write + Send>>,
    pending: Pending,
    next: AtomicU32,
    closed: Arc<AtomicBool>,
    extensions: Vec<String>,
}

/// A request in flight.
pub struct Ticket(Receiver<Result<Reply>>);

impl Ticket {
    fn wait(self) -> Result<Reply> {
        self.0.recv_timeout(REPLY_TIMEOUT).map_err(|_| Error::Closed)?
    }
}

fn parse(packet: &[u8]) -> Result<(u32, Reply)> {
    let mut reader = Reader { bytes: packet, offset: 0 };
    let kind = reader.u8()?;
    let id = reader.u32()?;
    let reply = match kind {
        STATUS => {
            let code = reader.u32()?;
            let message = if reader.done() { String::new() } else { reader.text()? };
            Reply::Status(code, message)
        }
        HANDLE => Reply::Handle(reader.bytes()?.to_vec()),
        DATA => Reply::Data(reader.bytes()?.to_vec()),
        ATTRS => Reply::Attrs(reader.attributes()?),
        NAME => {
            let count = reader.u32()? as usize;
            if count > MAX_ENTRIES {
                return Err(Error::Protocol("SFTP 服务器返回的目录项太多。".into()));
            }
            let mut names = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                let name = reader.text()?;
                reader.bytes()?;
                names.push(Entry { name, attributes: reader.attributes()? });
            }
            Reply::Names(names)
        }
        EXTENDED_REPLY => Reply::Extended,
        other => return Err(Error::Protocol(format!("意外的 SFTP 回复类型 {other}。"))),
    };
    Ok((id, reply))
}

fn read_packet(output: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut length = [0; 4];
    output.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_PACKET {
        return Err(std::io::Error::other("SFTP 服务器发送了无效的数据包大小。"));
    }
    let mut packet = vec![0; length];
    output.read_exact(&mut packet)?;
    Ok(packet)
}

impl Client {
    /// Performs the version handshake, then serves replies on a reader thread.
    pub fn start(mut input: Box<dyn Write + Send>, mut output: Box<dyn Read + Send>) -> Result<Self> {
        let init = { let mut bytes = vec![0, 0, 0, 5, INIT]; bytes.extend_from_slice(&3u32.to_be_bytes()); bytes };
        input.write_all(&init).and_then(|_| input.flush()).map_err(|_| Error::Closed)?;
        let version = read_packet(&mut output).map_err(|error| Error::Protocol(error.to_string()))?;
        let mut reader = Reader { bytes: &version, offset: 0 };
        if reader.u8()? != VERSION {
            return Err(Error::Protocol("SFTP 服务器没有回复版本。".into()));
        }
        let number = reader.u32()?;
        if number < 3 {
            return Err(Error::Protocol(format!("SFTP 服务器只支持协议版本 {number}。")));
        }
        let mut extensions = Vec::new();
        while !reader.done() {
            extensions.push(reader.text()?);
            reader.text()?;
        }
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        {
            let pending = pending.clone();
            let closed = closed.clone();
            std::thread::Builder::new()
                .name("omniterm-sftp".into())
                .spawn(move || {
                    loop {
                        let result = read_packet(&mut output).map_err(|_| Error::Closed).and_then(|packet| parse(&packet));
                        match result {
                            Ok((id, reply)) => {
                                if let Some(sender) = pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id) {
                                    let _ = sender.send(Ok(reply));
                                }
                            }
                            Err(error) => {
                                closed.store(true, Ordering::Release);
                                for (_, sender) in pending.lock().unwrap_or_else(|p| p.into_inner()).drain() {
                                    let _ = sender.send(Err(error.clone()));
                                }
                                return;
                            }
                        }
                    }
                })
                .map_err(|_| Error::Closed)?;
        }
        Ok(Self { input: Mutex::new(input), pending, next: AtomicU32::new(1), closed, extensions })
    }

    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn send(&self, build: impl FnOnce(u32) -> Packet) -> Result<Ticket> {
        if self.closed() {
            return Err(Error::Closed);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = channel();
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(id, sender);
        let packet = build(id).finish();
        let mut input = self.input.lock().unwrap_or_else(|p| p.into_inner());
        if input.write_all(&packet).and_then(|_| input.flush()).is_err() {
            self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
            return Err(Error::Closed);
        }
        Ok(Ticket(receiver))
    }

    fn call(&self, build: impl FnOnce(u32) -> Packet) -> Result<Reply> {
        match self.send(build)?.wait()? {
            Reply::Status(code, message) if code != STATUS_OK && code != STATUS_EOF => Err(Error::Status(code, message)),
            reply => Ok(reply),
        }
    }

    fn ok(&self, build: impl FnOnce(u32) -> Packet) -> Result<()> {
        match self.call(build)? {
            Reply::Status(STATUS_OK, _) | Reply::Extended => Ok(()),
            Reply::Status(code, message) => Err(Error::Status(code, message)),
            _ => Err(Error::Protocol("意外的 SFTP 回复。".into())),
        }
    }

    pub fn realpath(&self, path: &str) -> Result<String> {
        match self.call(|id| Packet::new(REALPATH, id).text(path))? {
            Reply::Names(names) => names.into_iter().next().map(|entry| entry.name).ok_or_else(|| Error::Protocol("REALPATH 没有返回路径。".into())),
            _ => Err(Error::Protocol("意外的 SFTP 回复。".into())),
        }
    }

    fn attrs(&self, kind: u8, path: &str) -> Result<Attributes> {
        match self.call(|id| Packet::new(kind, id).text(path))? {
            Reply::Attrs(attributes) => Ok(attributes),
            Reply::Status(code, message) => Err(Error::Status(code, message)),
            _ => Err(Error::Protocol("意外的 SFTP 回复。".into())),
        }
    }

    pub fn stat(&self, path: &str) -> Result<Attributes> {
        self.attrs(STAT, path)
    }

    pub fn lstat(&self, path: &str) -> Result<Attributes> {
        self.attrs(LSTAT, path)
    }

    pub fn exists(&self, path: &str) -> Result<bool> {
        match self.lstat(path) {
            Ok(_) => Ok(true),
            Err(Error::Status(NO_SUCH_FILE, _)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn mkdir(&self, path: &str) -> Result<()> {
        self.ok(|id| Packet::new(MKDIR, id).text(path).mode(Some(0o755)))
    }

    pub fn rmdir(&self, path: &str) -> Result<()> {
        self.ok(|id| Packet::new(RMDIR, id).text(path))
    }

    pub fn remove(&self, path: &str) -> Result<()> {
        self.ok(|id| Packet::new(REMOVE, id).text(path))
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.ok(|id| Packet::new(RENAME, id).text(from).text(to))
    }

    /// Replaces the destination atomically where the server allows it. Plain SFTP v3 rename
    /// refuses an existing destination, so the fallback removes it first.
    pub fn replace(&self, from: &str, to: &str) -> Result<()> {
        if self.extensions.iter().any(|name| name == "posix-rename@openssh.com") {
            return self.ok(|id| Packet::new(EXTENDED, id).text("posix-rename@openssh.com").text(from).text(to));
        }
        if self.exists(to)? {
            self.remove(to)?;
        }
        self.rename(from, to)
    }

    fn handle(&self, build: impl FnOnce(u32) -> Packet) -> Result<Vec<u8>> {
        match self.call(build)? {
            Reply::Handle(handle) => Ok(handle),
            Reply::Status(code, message) => Err(Error::Status(code, message)),
            _ => Err(Error::Protocol("意外的 SFTP 回复。".into())),
        }
    }

    fn close_handle(&self, handle: &[u8]) {
        let _ = self.ok(|id| Packet::new(CLOSE, id).bytes(handle));
    }

    pub fn readdir(&self, path: &str) -> Result<Vec<Entry>> {
        let handle = self.handle(|id| Packet::new(OPENDIR, id).text(path))?;
        let mut entries = Vec::new();
        let result = loop {
            match self.call(|id| Packet::new(READDIR, id).bytes(&handle)) {
                Ok(Reply::Names(batch)) => {
                    entries.extend(batch.into_iter().filter(|entry| entry.name != "." && entry.name != ".."));
                    if entries.len() > MAX_ENTRIES {
                        break Err(Error::Protocol("远程目录中的项目太多，无法列出。".into()));
                    }
                }
                Ok(Reply::Status(STATUS_EOF, _)) => break Ok(entries),
                Ok(_) => break Err(Error::Protocol("意外的 SFTP 回复。".into())),
                Err(error) => break Err(error),
            }
        };
        self.close_handle(&handle);
        result
    }

    /// Downloads into `local`, which must not exist. Reports bytes as they arrive.
    pub fn download(&self, remote: &str, local: &std::path::Path, mode: Option<u32>, canceled: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<()> {
        let handle = self.handle(|id| Packet::new(OPEN, id).text(remote).u32(OPEN_READ).u32(0))?;
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(mode.unwrap_or(0o644) & 0o777);
            }
            #[cfg(not(unix))]
            let _ = mode;
            let mut file = options.open(local).map_err(|error| Error::Protocol(error.to_string()))?;
            // Chunks are requested in order and written in order, `WINDOW` at a time; a short
            // read is completed before moving on, so the file never has a hole.
            let mut queue: VecDeque<(u64, u32, Ticket)> = VecDeque::new();
            let mut next: u64 = 0;
            let mut written: u64 = 0;
            loop {
                while queue.len() < WINDOW {
                    if canceled.load(Ordering::Relaxed) {
                        return Err(Error::Canceled);
                    }
                    let offset = next;
                    queue.push_back((offset, CHUNK, self.send(|id| Packet::new(READ, id).bytes(&handle).u64(offset).u32(CHUNK))?));
                    next += CHUNK as u64;
                }
                let Some((offset, length, ticket)) = queue.pop_front() else { return Ok(()) };
                match ticket.wait()? {
                    Reply::Data(data) => {
                        if offset != written {
                            return Err(Error::Protocol("SFTP 读取顺序错乱。".into()));
                        }
                        file.write_all(&data).map_err(|error| Error::Protocol(error.to_string()))?;
                        written += data.len() as u64;
                        progress(data.len() as u64);
                        if (data.len() as u32) < length {
                            // A short read: fetch the rest of this chunk before anything later.
                            let rest = length - data.len() as u32;
                            let start = written;
                            let follow = self.send(|id| Packet::new(READ, id).bytes(&handle).u64(start).u32(rest))?;
                            queue.push_front((start, rest, follow));
                        }
                    }
                    Reply::Status(STATUS_EOF, _) => {
                        // Later chunks are past the end as well.
                        for (_, _, ticket) in queue.drain(..) {
                            let _ = ticket.wait();
                        }
                        return Ok(());
                    }
                    Reply::Status(code, message) => return Err(Error::Status(code, message)),
                    _ => return Err(Error::Protocol("意外的 SFTP 回复。".into())),
                }
            }
        })();
        self.close_handle(&handle);
        result
    }

    pub fn upload(&self, local: &std::path::Path, remote: &str, mode: Option<u32>, canceled: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<()> {
        let mut file = std::fs::File::open(local).map_err(|error| Error::Protocol(error.to_string()))?;
        let handle = self.handle(|id| Packet::new(OPEN, id).text(remote).u32(OPEN_WRITE | OPEN_CREATE | OPEN_TRUNCATE).mode(mode))?;
        let result = (|| {
            let mut queue: VecDeque<(usize, Ticket)> = VecDeque::new();
            let mut offset: u64 = 0;
            let mut buffer = vec![0; CHUNK as usize];
            let mut finished = false;
            loop {
                while !finished && queue.len() < WINDOW {
                    if canceled.load(Ordering::Relaxed) {
                        return Err(Error::Canceled);
                    }
                    let mut filled = 0;
                    while filled < buffer.len() {
                        match file.read(&mut buffer[filled..]) {
                            Ok(0) => break,
                            Ok(read) => filled += read,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(error) => return Err(Error::Protocol(error.to_string())),
                        }
                    }
                    if filled == 0 {
                        finished = true;
                        break;
                    }
                    let at = offset;
                    let chunk = &buffer[..filled];
                    queue.push_back((filled, self.send(|id| Packet::new(WRITE, id).bytes(&handle).u64(at).bytes(chunk))?));
                    offset += filled as u64;
                    finished = filled < buffer.len();
                }
                let Some((length, ticket)) = queue.pop_front() else { return Ok(()) };
                match ticket.wait()? {
                    Reply::Status(STATUS_OK, _) => progress(length as u64),
                    Reply::Status(code, message) => return Err(Error::Status(code, message)),
                    _ => return Err(Error::Protocol("意外的 SFTP 回复。".into())),
                }
            }
        })();
        self.close_handle(&handle);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_carry_their_length_type_and_id() {
        let packet = Packet::new(REALPATH, 7).text(".").finish();
        assert_eq!(packet, [0, 0, 0, 10, REALPATH, 0, 0, 0, 7, 0, 0, 0, 1, b'.']);
    }

    #[test]
    fn replies_parse_names_and_attributes() {
        let mut body = vec![NAME, 0, 0, 0, 3, 0, 0, 0, 1];
        let push_text = |body: &mut Vec<u8>, text: &str| {
            body.extend_from_slice(&(text.len() as u32).to_be_bytes());
            body.extend_from_slice(text.as_bytes());
        };
        push_text(&mut body, "dir");
        push_text(&mut body, "drwxr-xr-x dir");
        body.extend_from_slice(&(ATTR_SIZE | ATTR_PERMISSIONS | ATTR_TIMES).to_be_bytes());
        body.extend_from_slice(&4096u64.to_be_bytes());
        body.extend_from_slice(&(S_IFDIR | 0o755).to_be_bytes());
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&1_700_000_000u32.to_be_bytes());
        let (id, reply) = parse(&body).unwrap();
        assert_eq!(id, 3);
        let Reply::Names(names) = reply else { panic!() };
        assert_eq!(names[0].name, "dir");
        assert!(names[0].attributes.directory());
        assert_eq!(names[0].attributes.mtime, Some(1_700_000_000));
    }

    #[test]
    fn truncated_packets_are_errors_not_panics() {
        assert!(parse(&[NAME, 0, 0, 0, 1, 0, 0, 0, 9]).is_err());
        assert!(parse(&[STATUS]).is_err());
    }
}
