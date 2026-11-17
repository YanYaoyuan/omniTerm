// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Offline diagnostics for the accessclient:// envelope found in the user's
//! AccessClient 4.6.20 installer. This decodes data; it never fetches a session,
//! follows an embedded URL, executes a client command, or logs credential values.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use flate2::bufread::ZlibDecoder;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

// The inspected loader allocates a 2 MiB decompression buffer. Bound input too,
// and stop after one extra output byte rather than expanding a compression bomb.
pub const MAX_WIRE_BYTES: usize = 4 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const SCHEME: &str = "accessclient://";

// No Serialize/Debug: the document may contain live tickets or credentials.
pub struct Envelope {
    document: Value,
    decoded_bytes: usize,
}

/// Only constants and counts may leave the decoder. Even unknown JSON keys can
/// contain private data, so diagnostic output uses a fixed whitelist of names.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    schema_version: u8,
    mode: &'static str,
    decoded_bytes: usize,
    root_type: &'static str,
    root_members: usize,
    recognized_fields: BTreeSet<&'static str>,
    recognized_field_types: BTreeMap<&'static str, &'static str>,
    nonempty_recognized_fields: BTreeSet<&'static str>,
    unknown_field_count: usize,
    credential_hints: BTreeSet<&'static str>,
    requested_clients: BTreeSet<&'static str>,
    protocol_hints: BTreeSet<&'static str>,
    contains_authentication_material: bool,
    references_web_request: bool,
    references_proxy: bool,
}

const KNOWN_FIELDS: &[&str] = &[
    "app",
    "args",
    "host",
    "port",
    "token",
    "sid",
    "header",
    "is_batch",
    "url_path",
    "use_ssl",
    "batch_url",
    "scsession_url_path",
    "opc_use_ssl",
    "opc_port",
    "opc_host",
    "opc_header",
    "proxy",
    "proxy.addr",
    "proxy_protocol",
    "sessionData",
    "session_data",
    "guiservice_token",
    "wsurl",
    "websid",
    "SessionName",
    "SessionHost",
    "SessionAccount",
    "SessionTitle",
    "Proto",
    "HostName",
    "PortNumber",
    "UserName",
    "Password",
    "Environment",
    "proto",
    "user",
    "mode",
    // Short names from the inspected loader's parameter translation table.
    "sn",
    "sh",
    "sa",
    "st",
    "dw",
    "dh",
    "as",
    "fa",
    "ac",
    "au",
    "sc",
    "bpp",
    "rd",
    "dr",
    "rm",
    "ec",
    "pc",
    "al",
    "rp",
    "rn",
    "rc",
    "mv",
    "co",
    "h",
    "sec",
    "p",
    "md",
    "tok",
    "hk",
    "it",
    "ns",
    "wp",
    "efs",
    "ki",
    "cc",
    "cl",
    "pr",
    "hn",
    "pn",
    "cp",
    "wt",
    "env",
    "un",
    "pw",
    "ws",
    "th",
    "tw",
    "nrwt",
    "otp",
    "path",
    "ver",
    "shterm_ver",
    "url",
    "control_mode",
    "pwd",
    "MODE",
    "SID",
    "TOKEN",
    "PWD",
    "secLayer",
    "remotefile",
];

fn nonempty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(_) => true,
    }
}

fn percent_decode(input: &[u8]) -> Result<Vec<u8>, &'static str> {
    let hex = |c: u8| (c as char).to_digit(16).map(|value| value as u8);
    let mut output = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            let (Some(high), Some(low)) = (input.get(i + 1).and_then(|c| hex(*c)), input.get(i + 2).and_then(|c| hex(*c))) else {
                return Err("AccessClient 链接包含无效的 URL 编码。");
            };
            output.push(high * 16 + low);
            i += 3;
        } else {
            // This is not an HTML form: '+' is part of the Base64 alphabet.
            output.push(input[i]);
            i += 1;
        }
    }
    Ok(output)
}

impl Envelope {
    pub fn decode(input: &str) -> Result<Self, &'static str> {
        if input.len() > MAX_WIRE_BYTES {
            return Err("AccessClient 链接超过诊断大小限制。");
        }
        let input = input.trim();
        let input = input.strip_prefix('"').and_then(|value| value.strip_suffix('"')).unwrap_or(input);
        if input.chars().any(char::is_control) {
            return Err("AccessClient 链接包含控制字符。");
        }
        let encoded = input.strip_prefix(SCHEME).ok_or("需要 accessclient:// 启动链接。").and_then(|value| {
            if value.is_empty() {
                Err("AccessClient 启动链接为空。")
            } else {
                Ok(value)
            }
        })?;
        let encoded = percent_decode(encoded.as_bytes())?;
        let compressed = [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD]
            .into_iter()
            .find_map(|engine| engine.decode(&encoded).ok())
            .ok_or("AccessClient 启动内容不是有效的 Base64 数据。")?;
        let mut decoder = ZlibDecoder::new(compressed.as_slice());
        let mut json = Vec::new();
        decoder
            .by_ref()
            .take((MAX_JSON_BYTES + 1) as u64)
            .read_to_end(&mut json)
            .map_err(|_| "AccessClient 启动内容无法按 zlib 解压。")?;
        if json.len() > MAX_JSON_BYTES {
            return Err("AccessClient 解压内容超过 2 MiB 限制。");
        }
        if decoder.total_in() as usize != compressed.len() {
            return Err("AccessClient 启动内容包含多余的压缩数据。");
        }
        let document: Value = serde_json::from_slice(&json).map_err(|_| "AccessClient 启动内容不是有效的 JSON。")?;
        if !document.is_object() && !document.is_array() {
            return Err("AccessClient JSON 需要是对象或批量数组。");
        }
        Ok(Self {
            document,
            decoded_bytes: json.len(),
        })
    }

    pub fn into_document(self) -> Value {
        self.document
    }

    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            schema_version: 2,
            mode: "offline-inspection-only",
            decoded_bytes: self.decoded_bytes,
            root_type: if self.document.is_array() { "array" } else { "object" },
            root_members: self
                .document
                .as_object()
                .map(|value| value.len())
                .or_else(|| self.document.as_array().map(|value| value.len()))
                .unwrap_or(0),
            recognized_fields: BTreeSet::new(),
            recognized_field_types: BTreeMap::new(),
            nonempty_recognized_fields: BTreeSet::new(),
            unknown_field_count: 0,
            credential_hints: BTreeSet::new(),
            requested_clients: BTreeSet::new(),
            protocol_hints: BTreeSet::new(),
            contains_authentication_material: false,
            references_web_request: false,
            references_proxy: false,
        };
        let mut pending = vec![&self.document];
        while let Some(value) = pending.pop() {
            match value {
                Value::Array(items) => pending.extend(items),
                Value::Object(fields) => {
                    for (key, value) in fields {
                        if let Some(name) = KNOWN_FIELDS.iter().copied().find(|known| *known == key) {
                            summary.recognized_fields.insert(name);
                            summary.recognized_field_types.insert(
                                name,
                                match value {
                                    Value::Null => "null",
                                    Value::Bool(_) => "boolean",
                                    Value::Number(_) => "number",
                                    Value::String(_) => "string",
                                    Value::Array(_) => "array",
                                    Value::Object(_) => "object",
                                },
                            );
                            if nonempty(value) {
                                summary.nonempty_recognized_fields.insert(name);
                            }
                        } else {
                            summary.unknown_field_count += 1;
                        }
                        match key.to_ascii_lowercase().as_str() {
                            "token" | "password" | "sid" | "header" | "opc_header" | "guiservice_token" | "websid" | "sessiondata" | "session_data"
                            | "environment" | "pw" | "pwd" | "tok" | "otp" | "env" | "ws" => {
                                summary.contains_authentication_material |= nonempty(value);
                                if value.as_str().is_some_and(|text| text.starts_with("OTP1:")) {
                                    summary.credential_hints.insert("otp1-ticket");
                                }
                            }
                            "url_path" | "scsession_url_path" | "batch_url" => summary.references_web_request = true,
                            _ => {}
                        }
                        if matches!(key.as_str(), "proxy" | "proxy.addr" | "proxy_protocol" | "guiservice_token" | "wsurl") {
                            summary.references_proxy |= nonempty(value);
                        }
                        if key == "app"
                            && let Some(client) = value.as_str()
                        {
                            for known in [
                                "putty",
                                "xshell",
                                "securecrt",
                                "scrt",
                                "mobaxterm",
                                "winscp",
                                "filezilla",
                                "xftp",
                                "mstsc",
                                "guiviewer",
                                "guiplayer",
                                "Terminal",
                                "iTerm2",
                            ] {
                                if client == known {
                                    summary.requested_clients.insert(known);
                                }
                            }
                        }
                        if matches!(key.as_str(), "proto" | "Proto" | "pr" | "proxy_protocol")
                            && let Some(protocol) = value.as_str()
                        {
                            for known in ["ssh", "sftp", "telnet", "rdp", "vnc", "xdmcp", "websocket"] {
                                if protocol.eq_ignore_ascii_case(known) {
                                    summary.protocol_hints.insert(known);
                                }
                            }
                        }
                        pending.push(value);
                    }
                }
                _ => {}
            }
        }
        summary
    }
}

pub fn inspect(mut input: impl Read) -> Result<Summary, &'static str> {
    let mut wire = String::new();
    input
        .by_ref()
        .take((MAX_WIRE_BYTES + 1) as u64)
        .read_to_string(&mut wire)
        .map_err(|_| "无法读取 UTF-8 格式的 AccessClient 链接。")
        .and_then(|_| Envelope::decode(&wire).map(|value| value.summary()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use serde_json::json;
    use std::io::Write;

    fn encode(bytes: &[u8]) -> String {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        format!("{SCHEME}{}", STANDARD.encode(encoder.finish().unwrap()))
    }

    #[test]
    fn diagnostic_output_contains_structure_but_no_ticket_host_account_or_unknown_values() {
        let packet = json!({ "app": "putty", "host": "private-gateway.invalid", "header": "private-cookie",
            "url_path": "/private/path", "token": "private-ticket", "args": { "HostName": "private-asset.invalid", "UserName": "private-user", "Password": "private-password", "Proto": "SSH" },
            "proxy_protocol": "websocket", "private-key-must-not-be-printed": { "random": "private-value" } });
        let summary = inspect(encode(packet.to_string().as_bytes()).as_bytes()).unwrap();
        let output = serde_json::to_string(&summary).unwrap();
        assert!(!output.contains("private"), "diagnostics must not expose values or unknown keys");
        assert!(summary.contains_authentication_material && summary.references_web_request && summary.references_proxy);
        assert!(summary.requested_clients.contains("putty"));
        assert!(summary.protocol_hints.contains("ssh") && summary.protocol_hints.contains("websocket"));
    }

    #[test]
    fn legacy_short_names_reveal_structure_without_disclosing_credentials() {
        let packet = json!({ "app": "putty", "hn": "private-host", "pn": "60022", "un": "private-account", "pw": "OTP1:private-ticket", "pr": "ssh", "proxy.addr": "", "env": "private-session", "private-key": "private-value" });
        let summary = inspect(encode(packet.to_string().as_bytes()).as_bytes()).unwrap();
        assert_eq!(summary.recognized_field_types.get("pw"), Some(&"string"));
        assert!(summary.contains_authentication_material);
        assert!(summary.credential_hints.contains("otp1-ticket"));
        assert!(!summary.references_proxy, "an empty proxy field is not a configured proxy");
        assert!(!summary.nonempty_recognized_fields.contains("proxy.addr"));
        assert_eq!(summary.unknown_field_count, 1);
        assert!(summary.protocol_hints.contains("ssh"));
        assert!(!serde_json::to_string(&summary).unwrap().contains("private"));
    }

    #[test]
    fn percent_encoding_preserves_base64_and_supports_quoted_windows_invocations() {
        let wire = encode(br#"{"app":"winscp","args":{"proto":"sftp"}}"#);
        let percent: String = wire[SCHEME.len()..].bytes().map(|byte| format!("%{byte:02X}")).collect();
        let summary = inspect(format!("\"{SCHEME}{percent}\"\n").as_bytes()).unwrap();
        assert!(summary.requested_clients.contains("winscp"));
        assert!(summary.protocol_hints.contains("sftp"));
        assert_eq!(percent_decode(b"a+b%2Fc").unwrap(), b"a+b/c");
    }

    #[test]
    fn malformed_or_excessive_payloads_fail_without_echoing_input() {
        for wire in [
            "https://private-ticket",
            "accessclient://",
            "accessclient://%zz",
            "accessclient://private-ticket",
            "accessclient://YWJj",
            &encode(b"private-ticket"),
            &encode(b"true"),
        ] {
            let error = inspect(wire.as_bytes()).unwrap_err();
            assert!(!error.contains("private-ticket"));
        }
        let huge = encode(&vec![b' '; MAX_JSON_BYTES + 1]);
        assert!(inspect(huge.as_bytes()).unwrap_err().contains("2 MiB"));
        let wire = "x".repeat(MAX_WIRE_BYTES + 1);
        assert!(inspect(wire.as_bytes()).is_err());
    }

    #[test]
    fn inspecting_batch_or_arbitrary_client_names_never_runs_a_command() {
        let wire = encode(br#"[{"app":"calc.exe; private-command","args":{"Password":"private-secret"}},{"app":"xftp","proto":"sftp"}]"#);
        let summary = inspect(wire.as_bytes()).unwrap();
        assert_eq!(summary.root_type, "array");
        assert_eq!(summary.root_members, 2);
        assert_eq!(summary.requested_clients.into_iter().collect::<Vec<_>>(), vec!["xftp"]);
    }
}
