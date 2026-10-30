//! Wire format between Electron and this process.
//!
//! Requests arrive as one JSON object per line on stdin. Everything leaves through stdout as
//! `u32 LE length | u8 kind | payload` frames, so terminal bytes need no JSON escaping. The
//! bounded channel gives backpressure: a slow consumer stalls the PTY readers, and with them
//! the producing programs, instead of growing memory here.

use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::io::Write;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::sync_channel;

const KIND_JSON: u8 = 1;
const KIND_DATA: u8 = 2;
const QUEUE_FRAMES: usize = 64;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub enum Frame {
    Json(Value),
    Data { session_id: String, seq_end: u64, bytes: Vec<u8> },
}

impl Frame {
    fn encode(&self, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(&[0; 4]);
        match self {
            Self::Json(value) => {
                out.push(KIND_JSON);
                // Values built in this process always serialize.
                let _ = serde_json::to_writer(&mut *out, value);
            }
            Self::Data { session_id, seq_end, bytes } => {
                out.push(KIND_DATA);
                out.push(session_id.len() as u8);
                out.extend_from_slice(session_id.as_bytes());
                out.extend_from_slice(&seq_end.to_le_bytes());
                out.extend_from_slice(bytes);
            }
        }
        let length = (out.len() - 4) as u32;
        out[..4].copy_from_slice(&length.to_le_bytes());
    }
}

#[derive(Clone)]
pub struct Outbox(SyncSender<Frame>);

impl Outbox {
    pub fn channel() -> (Self, Receiver<Frame>) {
        let (sender, receiver) = sync_channel(QUEUE_FRAMES);
        (Self(sender), receiver)
    }

    pub fn send(&self, frame: Frame) {
        // The receiver only ends when the process is shutting down.
        let _ = self.0.send(frame);
    }

    pub fn event(&self, event: &str, params: Value) {
        self.send(Frame::Json(json!({ "event": event, "params": params })));
    }

    pub fn respond(&self, id: u64, result: Result<Value, String>) {
        self.send(Frame::Json(match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(message) => json!({ "id": id, "error": { "message": message } }),
        }));
    }
}

/// Runs until stdout closes. The whole write is one syscall sequence on one thread.
pub fn write_frames(receiver: Receiver<Frame>, mut output: impl Write) {
    let mut buffer = Vec::with_capacity(64 * 1024);
    while let Ok(frame) = receiver.recv() {
        frame.encode(&mut buffer);
        if output.write_all(&buffer).is_err() || output.flush().is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_frames_carry_raw_bytes_after_a_length_prefix() {
        let mut out = Vec::new();
        Frame::Data { session_id: "ab".to_string(), seq_end: 7, bytes: vec![0, 255] }.encode(&mut out);
        assert_eq!(u32::from_le_bytes([out[0], out[1], out[2], out[3]]) as usize, out.len() - 4);
        assert_eq!(out[4], KIND_DATA);
        assert_eq!(&out[5..8], &[2, b'a', b'b']);
        assert_eq!(u64::from_le_bytes(out[8..16].try_into().expect("eight bytes")), 7);
        assert_eq!(&out[16..], &[0, 255]);
    }

    #[test]
    fn json_frames_decode_back() {
        let mut out = Vec::new();
        Frame::Json(json!({ "id": 1 })).encode(&mut out);
        assert_eq!(out[4], KIND_JSON);
        let value: Value = serde_json::from_slice(&out[5..]).expect("valid json");
        assert_eq!(value["id"], 1);
    }
}
