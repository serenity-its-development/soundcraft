//! The OneOS host bridge: `postMessage` to the ONE parent the frame page declared. The frame page's loader
//! (`bootstrap.js`) sets `globalThis.__craftHost = {app, version, parent}` from the page's `data-parent`; the editor
//! lives on its own origin, so the parent is a DIFFERENT origin. Messages go to that origin only, and only the parent
//! window on that origin is heard. Every message is a plain object with a `type`; `session` is the per-open nonce the
//! host mints in `craft:open` and every later message echoes
//! (`plans/mona/craft-integration-design-2026-10-25.md` section 2.2):
//!
//! - `craft:ready {app, version}` (to the parent): listening; send the file now.
//! - `craft:open {session, name, mime, bytes, versionId, readOnly, collab}` (from the parent): open these bytes (a WAV, AIFF or FLAC file, or a Sound session bundle `.scraftz`).
//! - `craft:save {session, name, mime, bytes, baseVersionId, saveAs}` (to the parent): the person saved. The bytes are a
//!   transferred `ArrayBuffer`; the host makes a NEW version of the file through the compare-and-set and answers
//!   `craft:saved {session, versionId}` (the next save's base) or `craft:refused {session, code, reason}`. Anything that is not the
//!   opened file under its own name (a bounce to another name, a stem, a Save As) goes as `saveAs: true`, a copy, never a new version
//!   of the opened file.
//! - `craft:dirty {session, dirty}` (to the parent): unsaved changes appeared or went away.
//! - `craft:failed {error}` (to the parent): the editor could not start.
//! - `craft:opened {session}` / `craft:view {session, scrollTop, scrollHeight, clientHeight, selection ...}` / `craft:viewed {session}`:
//!   the progressive open's hand-over (`handoff.rs`).
//! - `craft:command {session, id, method: "command.batch", params: {label, steps: [{name, args?}]}}` (from the parent) is the
//!   assistant's allow-listed run (`soundcraft_engine::host_commands`): all of it as ONE undo step, or none of it. It is answered
//!   `craft:result {session, id, ok:true, result}` or `{ok:false, error: "<code>: <reason>"}` (a command outside the list, a step the
//!   session forbids, a file open read-only, or a `session` that is not the open one).
//!
//! The editor never talks to a server of its own: there is no network call in this module, and the file's bytes arrive and leave
//! only as these messages. A file opened read-only is never saved (the save fails by name).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use soundcraft_ui_egui::SoundApp;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::js_sys;

/// Longest file name taken from the parent page.
const MAX_NAME_CHARS: usize = 255;
/// The most bytes a file the host sends may be (the host refuses a larger file before any byte moves; this is the editor's own bound).
const MAX_OPEN_BYTES: u32 = 200 * 1024 * 1024;

/// What the host's last `craft:open` said.
#[derive(Default)]
struct Opened {
    session: String,
    name: String,
    version_id: String,
    read_only: bool,
}

/// An assistant run the host relayed, waiting for the next frame.
pub struct Run {
    pub id: f64,
    pub params: serde_json::Value,
}

pub type Runs = Rc<RefCell<Vec<Run>>>;

/// Files the host sent, waiting for the next frame: `(name, bytes)`.
pub type Opens = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

thread_local! {
    /// The bridge [`deliver`] posts through (the engine's hook is a plain `fn`).
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    /// The declared parent origin: the only target and the only sender.
    origin: String,
    app: String,
    version: String,
    opened: Rc<RefCell<Opened>>,
}

/// The unsaved-changes state last reported to the parent.
#[derive(Default)]
pub struct DirtyReport(Cell<Option<(String, bool)>>);

/// `scheme://host[:port]` exactly: https, or http only on localhost (the dev estate).
fn plain_origin(v: &str) -> bool {
    let Some((scheme, rest)) = v.split_once("://") else { return false };
    let local = rest == "localhost" || rest.starts_with("localhost:") || rest.starts_with("127.0.0.1");
    (scheme == "https" || (scheme == "http" && local)) && !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
}

impl Bridge {
    /// `None` when the page is not framed, or declares no parent origin.
    pub fn new() -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let declared = js_sys::Reflect::get(&window, &JsValue::from_str("__craftHost")).ok().filter(|d| d.is_object())?;
        let origin = string(&declared, "parent").filter(|o| plain_origin(o))?;
        Some(Self {
            window,
            parent,
            origin,
            app: string(&declared, "app").unwrap_or_else(|| "sound".into()),
            version: string(&declared, "version").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
            opened: Rc::default(),
        })
    }

    /// Make this the bridge [`deliver`] uses.
    pub fn install(&self) {
        BRIDGE.with(|b| *b.borrow_mut() = Some(self.clone()));
    }

    fn post(&self, message: &js_sys::Object, transfer: Option<&JsValue>) -> Result<(), String> {
        let sent = match transfer {
            Some(t) => self.parent.post_message_with_transfer(message, &self.origin, &js_sys::Array::of1(t)),
            None => self.parent.post_message(message, &self.origin),
        };
        sent.map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
    }

    pub fn post_ready(&self) {
        if let Some(m) = message(&[("type", "craft:ready".into()), ("app", self.app.as_str().into()), ("version", self.version.as_str().into())]) {
            let _ = self.post(&m, None);
        }
    }

    /// `craft:opened {session}`: the session is loaded and drawn, the editor takes input (handoff.rs).
    pub fn post_opened(&self) {
        let session = self.opened.borrow().session.clone();
        if let Some(m) = message(&[("type", "craft:opened".into()), ("session", session.as_str().into())]) {
            let _ = self.post(&m, None);
        }
    }

    /// `craft:viewed {session}`: the first view's place is applied and drawn; the host may let the first view go.
    pub fn post_viewed(&self) {
        let session = self.opened.borrow().session.clone();
        if let Some(m) = message(&[("type", "craft:viewed".into()), ("session", session.as_str().into())]) {
            let _ = self.post(&m, None);
        }
    }

    /// The app couldn't start (no WebGL, for example): let the parent say so.
    pub fn post_failed(&self, error: &str) {
        if let Some(m) = message(&[("type", "craft:failed".into()), ("error", error.into())]) {
            let _ = self.post(&m, None);
        }
    }

    /// `craft:result {session, id, ok, result|error}` to the parent.
    pub fn post_result(&self, id: f64, outcome: Result<serde_json::Value, String>) {
        let session = self.opened.borrow().session.clone();
        let mut fields: Vec<(&str, JsValue)> =
            vec![("type", "craft:result".into()), ("session", session.as_str().into()), ("id", id.into()), ("ok", outcome.is_ok().into())];
        match &outcome {
            Ok(value) => {
                let result = js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL);
                fields.push(("result", result));
            }
            Err(error) => fields.push(("error", error.as_str().into())),
        }
        if let Some(m) = message(&fields) {
            let _ = self.post(&m, None);
        }
    }

    /// Hear the host: `craft:open` queues the file like a picked one; `craft:saved` and `craft:refused` answer a save;
    /// `craft:command` queues an assistant run.
    pub fn listen(&self, opens: Opens, runs: Runs, notices: Rc<RefCell<Vec<String>>>, ctx: egui::Context) {
        let bridge = self.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != bridge.origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &bridge.parent)) {
                return;
            }
            let data = event.data();
            let Some(kind) = string(&data, "type") else { return };
            match kind.as_str() {
                "craft:open" => {
                    let Some(session) = string(&data, "session").filter(|s| !s.is_empty()) else { return };
                    let name: String = string(&data, "name")
                        .filter(|n| !n.trim().is_empty())
                        .map(|n| n.chars().take(MAX_NAME_CHARS).collect())
                        .unwrap_or_else(|| "Sound.wav".to_string());
                    // Refuse an oversized file before copying it into the app's memory.
                    let Some(array) = byte_array(&data) else { return };
                    if array.length() > MAX_OPEN_BYTES {
                        notices.borrow_mut().push(format!("Open: {name} is larger than the {} MB Sound opens in the browser", MAX_OPEN_BYTES >> 20));
                        return;
                    }
                    *bridge.opened.borrow_mut() = Opened {
                        session,
                        name: name.clone(),
                        version_id: string(&data, "versionId").unwrap_or_default(),
                        read_only: boolean(&data, "readOnly"),
                    };
                    opens.borrow_mut().push((name, array.to_vec()));
                    crate::handoff::wasm::note_open();
                }
                // The loader keeps the place on `__craftFirstView`; the app reads it (handoff.rs) - this wakes it.
                "craft:view" => {}
                "craft:saved" => {
                    let mut opened = bridge.opened.borrow_mut();
                    if string(&data, "session").as_deref() == Some(opened.session.as_str())
                        && let Some(version) = string(&data, "versionId")
                    {
                        opened.version_id = version;
                    }
                }
                "craft:refused" => {
                    if string(&data, "session").as_deref() == Some(bridge.opened.borrow().session.as_str()) {
                        let reason = string(&data, "reason").unwrap_or_else(|| "refused".into());
                        notices.borrow_mut().push(format!("Couldn't save: {reason}"));
                    }
                }
                "craft:command" => {
                    let session = string(&data, "session").unwrap_or_default();
                    let id = js_sys::Reflect::get(&data, &JsValue::from_str("id")).ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let (current, read_only) = {
                        let opened = bridge.opened.borrow();
                        (opened.session.clone(), opened.read_only)
                    };
                    if session.is_empty() || session != current {
                        bridge.post_result(id, Err("session-mismatch: this command is for another open of the file".into()));
                    } else if read_only {
                        bridge.post_result(id, Err("read-only: this file is open to view; the assistant cannot change it".into()));
                    } else if string(&data, "method").as_deref() != Some("command.batch") {
                        bridge.post_result(id, Err("method-not-allowed: a run is one command.batch".into()));
                    } else {
                        let params = js_sys::Reflect::get(&data, &JsValue::from_str("params"))
                            .ok()
                            .and_then(|p| js_sys::JSON::stringify(&p).ok())
                            .and_then(|s| s.as_string())
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(serde_json::Value::Null);
                        runs.borrow_mut().push(Run { id, params });
                    }
                }
                _ => {}
            }
            ctx.request_repaint();
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// After each frame: tell the parent when unsaved changes appear or go away.
    pub fn report_dirty(&self, app: &SoundApp, reported: &DirtyReport) {
        let session = self.opened.borrow().session.clone();
        if session.is_empty() {
            return;
        }
        let dirty = app.engine.is_dirty();
        let now = (session.clone(), dirty);
        let last = reported.0.take();
        if last.as_ref() == Some(&now) {
            reported.0.set(last);
            return;
        }
        let sent = message(&[("type", "craft:dirty".into()), ("session", session.as_str().into()), ("dirty", dirty.into())]).is_some_and(|m| self.post(&m, None).is_ok());
        reported.0.set(if sent { Some(now) } else { last });
    }

    /// The person saved or bounced `name`: the opened file under its own name is a new version of it, anything else a copy.
    pub fn post_save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let mime = mime_of(name);
        let (session, base, as_copy) = {
            let opened = self.opened.borrow();
            if opened.session.is_empty() {
                return Err("the host has not opened a session yet".to_string());
            }
            if opened.read_only {
                return Err("this file is open to view; saving is off".to_string());
            }
            (opened.session.clone(), opened.version_id.clone(), name != opened.name)
        };
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let m = message(&[
            ("type", "craft:save".into()),
            ("session", session.as_str().into()),
            ("name", name.into()),
            ("mime", mime.into()),
            ("baseVersionId", base.as_str().into()),
            ("saveAs", as_copy.into()),
            ("bytes", buffer.clone().into()),
        ])
        .ok_or_else(|| "couldn't build the message".to_string())?;
        self.post(&m, Some(&buffer))
    }
}

impl crate::handoff::Link for Bridge {
    fn open_seq(&self) -> u64 {
        crate::handoff::wasm::opens()
    }
    fn take_view(&self) -> Option<crate::handoff::FirstView> {
        crate::handoff::wasm::take_view(&self.opened.borrow().session)
    }
    fn post_opened(&self) {
        Bridge::post_opened(self);
    }
    fn post_viewed(&self) {
        Bridge::post_viewed(self);
    }
}

/// The engine's delivery hook when framed ([`soundcraft_engine::io::set_deliver`]): hand the file to the host. `Ok` once it was posted.
pub fn deliver(name: &str, bytes: &[u8]) -> Result<(), String> {
    let bridge = BRIDGE.with(|b| b.borrow().clone()).ok_or("the page that embeds Sound is gone")?;
    bridge.post_save(name, bytes)
}

/// The media type a saved or bounced file goes under.
fn mime_of(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("wav") => "audio/wav",
        Some("aif" | "aiff") => "audio/aiff",
        Some("flac") => "audio/flac",
        Some("scraftz") => "application/x-soundcraft-session",
        Some("mid" | "midi") => "audio/midi",
        Some("txt") => "text/plain",
        Some("musicxml") => "application/vnd.recordare.musicxml+xml",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
}

fn boolean(data: &JsValue, key: &str) -> bool {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

/// `bytes` (an `ArrayBuffer` or `Uint8Array`) as a view, not yet copied.
fn byte_array(data: &JsValue) -> Option<js_sys::Uint8Array> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        Some(array.clone())
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(js_sys::Uint8Array::new(&bytes))
    } else {
        None
    }
}
