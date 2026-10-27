//! One Sound editor window's host side for node: the engine the browser build runs, so OneOS's `craft-sound-ai-drive.test.ts` can drive a
//! window through the SAME `command.batch` the browser build answers `craft:command` with (`soundcraft_engine::host_commands`), open
//! an audio file or a session bundle as the host does (`host_open`), and read back the file the editor saves.

use std::cell::RefCell;

use serde_json::{Value, json};
use soundcraft_engine::Engine;
use soundcraft_engine::io;
use wasm_bindgen::prelude::*;

thread_local! {
    /// What the engine last handed the host (`io::set_deliver`): the name and the bytes.
    static DELIVERED: RefCell<Option<(String, Vec<u8>)>> = const { RefCell::new(None) };
}

fn deliver(name: &str, bytes: &[u8]) -> Result<(), String> {
    DELIVERED.with(|d| *d.borrow_mut() = Some((name.to_string(), bytes.to_vec())));
    Ok(())
}

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// How the opened file is written back (the web shell's `HostKind`).
enum Kind {
    Session,
    Audio { format: &'static str, bit_depth: &'static str },
}

#[wasm_bindgen]
pub struct Window {
    engine: Engine,
    name: String,
    kind: Option<Kind>,
}

#[wasm_bindgen]
impl Window {
    /// A window with an empty session.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Window {
        io::set_deliver(deliver);
        Window { engine: Engine::default(), name: String::new(), kind: None }
    }

    /// Open what the host sent (`craft:open`): a session bundle, or an audio file as one track holding one clip. Answers how it
    /// will be saved back: `{kind: "session"}` or `{kind: "audio", format, bit_depth, rate}`.
    pub fn open_b64(&mut self, name: &str, b64: &str) -> Result<String, JsError> {
        let bytes = io_b64(b64)?;
        let lower = name.to_ascii_lowercase();
        let described = if lower.ends_with(".scraftz") || lower.ends_with(".scraft") {
            io::open_session_bytes(&mut self.engine, name, &bytes).map_err(err)?;
            self.kind = Some(Kind::Session);
            json!({ "kind": "session" })
        } else {
            let f = soundcraft_engine::host_open::open_audio_file(&mut self.engine, name, &bytes).map_err(err)?;
            self.kind = Some(Kind::Audio { format: f.format, bit_depth: f.bit_depth });
            json!({ "kind": "audio", "format": f.format, "bit_depth": f.bit_depth, "rate": f.rate })
        };
        self.name = name.to_string();
        Ok(described.to_string())
    }

    /// The assistant's run (`craft:command`, `command.batch`): the answer as JSON text, or an error `<code>: <reason>`.
    pub fn command_batch(&mut self, params_json: &str) -> Result<String, JsError> {
        let params: Value = serde_json::from_str(params_json).map_err(err)?;
        self.engine.run_command_batch(&params).map(|v| v.to_string()).map_err(|e| JsError::new(&e))
    }

    /// The person presses Save: the file the host is handed, as `{name, b64}` (the name is the opened file's own).
    pub fn save(&mut self) -> Result<String, JsError> {
        DELIVERED.with(|d| *d.borrow_mut() = None);
        match self.kind.as_ref().ok_or_else(|| JsError::new("the host has not opened a file yet"))? {
            Kind::Session => {
                self.engine.execute("session.save", &json!({ "path": self.name, "replace": true })).map_err(err)?;
            }
            Kind::Audio { format, bit_depth } => {
                self.engine
                    .execute("file.bounce_mix", &json!({ "path": self.name, "format": format, "bit_depth": bit_depth, "dither": false }))
                    .map_err(err)?;
                self.engine.mark_clean();
            }
        }
        let (name, bytes) = DELIVERED.with(|d| d.borrow_mut().take()).ok_or_else(|| JsError::new("the save handed the host nothing"))?;
        Ok(json!({ "name": name, "b64": b64(&bytes) }).to_string())
    }

    /// How many undo steps the open session holds.
    pub fn undo_depth(&self) -> usize {
        self.engine.undo_history().len()
    }

    /// The label of the last undo step.
    pub fn undo_label(&self) -> Option<String> {
        self.engine.undo_label().map(str::to_owned)
    }

    /// Undo one step (the person's Undo).
    pub fn undo(&mut self) -> bool {
        self.engine.undo()
    }

    /// Whether the session has unsaved changes.
    pub fn dirty(&self) -> bool {
        self.engine.is_dirty()
    }

    /// The session as a short JSON summary (`session.inspect`), the oracle a test reads back.
    pub fn inspect(&mut self) -> Result<String, JsError> {
        self.engine.execute("session.inspect", &json!({})).map(|v| v.to_string()).map_err(err)
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (with padding).
fn b64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

fn io_b64(s: &str) -> Result<Vec<u8>, JsError> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = TABLE.iter().position(|t| *t == c).ok_or_else(|| JsError::new("not base64"))? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}
