//! The assistant's command surface for the OneOS host bridge (`craft:command {method: "command.batch"}`).
//!
//! The host relays an allow-listed run: `{label, steps: [{name, args?}]}`. The whole run is judged BEFORE anything runs (an id
//! outside the list, or an argument that names a file or an address, refuses the run), every step runs through the same registry
//! the menus, the control channel and the MCP server use (so a step the session forbids is refused exactly as it would be for the
//! person), and the run is ONE undo step named by `label`: a step that fails takes back every step before it, so a run is all or
//! nothing. There is no file-path, dialog, transport or undo command in the list: files move only through the host, and a run edits
//! the session, it does not play it.
//!
//! This list is the one declared in the host's `platform-craft` registry as `sound` commands (pinned equal there).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::Engine;

/// Registry commands the assistant may run. Each acts at once on the session (no dialog, no file picker, no pointer gesture, no
/// playback), and none is Undo or Redo: undoing a run is the person's.
pub const ASSISTANT_COMMANDS: &[&str] = &[
    // Choosing what the next step acts on.
    "edit.select",
    "edit.select_all",
    "edit.select_none",
    // Cut, copy and arrange what is selected.
    "edit.cut",
    "edit.copy",
    "edit.paste",
    "edit.clear",
    "edit.duplicate",
    "edit.repeat",
    "edit.shift",
    "edit.insert_silence",
    "edit.separate",
    "edit.separate_on_grid",
    "edit.separate_at_transients",
    "edit.heal",
    "edit.consolidate",
    "edit.trim_to_selection",
    "edit.trim_start_to_insertion",
    "edit.trim_end_to_insertion",
    "edit.mute_clips",
    "edit.strip_silence",
    "edit.fades_create",
    "edit.fades_delete",
    "edit.move_clips",
    "edit.trim_clip",
    "edit.space_clips",
    "edit.tce_to_timeline",
    // Clips.
    "clip.rename",
    "clip.gain",
    "clip.color",
    "clip.loop",
    "clip.unloop",
    "clip.group",
    "clip.ungroup",
    "clip.gain_render",
    "clip.gain_bypass",
    "clip.effects_set",
    "clip.effects_bypass",
    "clip.effects_render",
    "clip.clear",
    // Processing the audio itself: normalize, reverse, invert, gain, stretch, pitch, speed, generate.
    "audiosuite.process",
    // Tracks.
    "track.new",
    "track.delete",
    "track.rename",
    "track.color",
    "track.duplicate",
    "track.move",
    "track.change_width",
    "track.split_into_mono",
    "track.make_inactive",
    // The mix.
    "mix.volume",
    "mix.pan",
    "mix.mute",
    "mix.solo",
    "mix.trim",
    "mix.phase_invert",
    "mix.new_bus",
    "mix.send",
    "mix.send_level",
    "mix.insert",
    "mix.insert_param",
    "mix.insert_params",
    "mix.insert_bypass",
    "mix.insert_remove",
    "automation.set_point",
    "automation.write_range",
    "automation.clear",
    "automation.thin",
    // Markers and the cursor.
    "markers.add",
    "markers.edit",
    "markers.delete",
    "transport.locate",
    // Read-only.
    "session.inspect",
    "clip.inspect",
    "track.inspect",
    "session.format_time",
    "session.audio_health",
];

/// Commands the window itself provides.
pub const ARGUMENT_COMMANDS: &[&str] = &[];

/// Every command id a run may name, in the order the host's registry declares them.
pub fn allowed() -> Vec<&'static str> {
    ASSISTANT_COMMANDS.iter().chain(ARGUMENT_COMMANDS).copied().collect()
}

const MAX_STEPS: usize = 50;
const MAX_LABEL: usize = 120;
/// Argument names that would point a command at a file or a network address: refused anywhere in a step's arguments.
const FORBIDDEN_ARGS: &[&str] = &["path", "paths", "dir", "folder", "file", "url", "uri", "href", "server", "data"];

/// The first forbidden argument name anywhere in `v`.
fn forbidden_arg(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.iter().find_map(|(k, x)| if FORBIDDEN_ARGS.contains(&k.as_str()) { Some(k.clone()) } else { forbidden_arg(x) }),
        Value::Array(a) => a.iter().find_map(forbidden_arg),
        _ => None,
    }
}

impl Engine {
    /// One `command.batch`: all of it, as ONE undo step, or none of it.
    pub fn run_command_batch(&mut self, params: &Value) -> Result<Value, String> {
        let label = params.get("label").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if label.chars().count() > MAX_LABEL {
            return Err(format!("label-too-long: a run's label is at most {MAX_LABEL} characters"));
        }
        let steps = params.get("steps").and_then(Value::as_array).ok_or("run-empty: a run states its steps")?;
        if steps.is_empty() {
            return Err("run-empty: a run states at least one command".into());
        }
        if steps.len() > MAX_STEPS {
            return Err(format!("run-too-long: a run is at most {MAX_STEPS} steps"));
        }
        // Judge the whole run first; nothing has run when one id is outside the list.
        let mut named = Vec::with_capacity(steps.len());
        for step in steps {
            let name = step.get("name").and_then(Value::as_str).unwrap_or("");
            if !ASSISTANT_COMMANDS.contains(&name) && !ARGUMENT_COMMANDS.contains(&name) {
                return Err(format!("command-not-allowed: Sound does not allow the command '{name}' -- nothing was run"));
            }
            let args = step.get("args").cloned().unwrap_or_else(|| json!({}));
            if let Some(k) = forbidden_arg(&args) {
                return Err(format!(
                    "argument-not-allowed: step '{name}' names '{k}', and a run does not reach files or addresses -- nothing was run"
                ));
            }
            named.push((name, args));
        }
        if self.transport.recording {
            return Err("recording: stop recording first; nothing was run".into());
        }
        let label = if label.is_empty() { "Assistant" } else { label };

        // What to put back when a step fails: the session, both history stacks, and the engine's own bookkeeping.
        let before = Arc::clone(&self.doc);
        let undo_len = self.undo.len();
        let redo_before = self.redo.clone();
        let (rev, dirty) = (self.revision, self.dirty);
        let journal_len = self.journal.len();
        let side_effects = (self.clipboard.clone(), self.messages.clone(), self.transport_requests.clone(), self.last_selection.clone());
        let limit = self.history_limit;
        // The steps inside are folded afterwards, so they must not push the oldest history out.
        self.history_limit = usize::MAX;
        self.merge_key = None;
        let mut results: Vec<Value> = Vec::with_capacity(named.len());
        let mut failure: Option<String> = None;
        for (i, (name, args)) in named.iter().enumerate() {
            match self.execute(name, args) {
                Ok(v) => results.push(json!({ "name": name, "result": v })),
                Err(e) => {
                    failure = Some(format!("step-failed: step {} ({name}): {e} -- nothing was changed", i + 1));
                    break;
                }
            }
        }
        self.history_limit = limit;
        self.merge_key = None;
        if let Some(why) = failure {
            self.doc = before;
            self.undo.truncate(undo_len);
            self.redo = redo_before;
            self.revision = rev.wrapping_add(1);
            self.dirty = dirty;
            self.journal.truncate(journal_len);
            (self.clipboard, self.messages, self.transport_requests, self.last_selection) = side_effects;
            return Err(why);
        }
        let mut folded = false;
        if self.undo.len() > undo_len {
            // The first step of the run holds the session as it was before the run: the folded step goes back to exactly there.
            self.undo.truncate(undo_len + 1);
            if let Some(first) = self.undo.get_mut(undo_len) {
                first.label = label.to_string();
            }
            if self.undo.len() > limit {
                let excess = self.undo.len() - limit;
                self.undo.drain(..excess);
            }
            folded = true;
        }
        Ok(json!({ "label": label, "steps": results, "undoStep": folded }))
    }
}
