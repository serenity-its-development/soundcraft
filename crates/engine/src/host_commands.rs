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

/// Registry commands the assistant may run: EVERY command of the registry except those in [`LEFT_OUT`], each with its reason.
/// The session's whole command registry (select, edit, clips, audiosuite, tracks, mix, automation, markers, MIDI events, tempo). A step the document's state forbids, or one that needs a dialog or a pointer, is refused by the registry exactly as it would be for
/// the person, and the run is taken back whole.
pub const ASSISTANT_COMMANDS: &[&str] = &[
    // edit
    "edit.copy_audio_as_midi",
    // audiosuite
    "audiosuite.process",
    // clip
    "clip.auto_rename",
    "clip.edit_lock",
    "clip.time_lock",
    "clip.send_to_back",
    "clip.bring_to_front",
    "clip.rating",
    "clip.group",
    "clip.ungroup",
    "clip.ungroup_all",
    "clip.loop",
    "clip.rename",
    "clip.gain",
    "clip.gain_nudge_up",
    "clip.gain_nudge_down",
    "clip.gain_render",
    "clip.gain_bypass",
    "clip.identify_sync_point",
    "clip.remove_sync_point",
    "clip.quantize_to_grid",
    "clip.color",
    "clip.elastic_properties",
    "clip.remove_warp",
    "clip.place_source",
    "clip.clear",
    "clip.regroup",
    "clip.unloop",
    "clip.effects_set",
    "clip.effects_bypass",
    "clip.effects_render",
    "clip.conform_to_tempo",
    "clip.remove_pitch_shift",
    // edit
    "edit.restore_last_selection",
    "edit.cut",
    "edit.copy",
    "edit.paste",
    "edit.clear",
    "edit.cut_clip_gain",
    "edit.clear_clip_gain",
    "edit.cut_all_automation",
    "edit.clear_all_automation",
    "edit.clear_pan_automation",
    "edit.clear_plugin_automation",
    "edit.paste_repeat_to_fill",
    "edit.select_all",
    "edit.select",
    "edit.select_none",
    "edit.selection_to_timeline",
    "edit.timeline_to_selection",
    "edit.move_selection_left",
    "edit.move_selection_right",
    "edit.halve_selection",
    "edit.double_selection",
    "edit.extend_selection_up",
    "edit.extend_selection_down",
    "edit.remove_selection_top",
    "edit.remove_selection_bottom",
    "edit.duplicate",
    "edit.repeat",
    "edit.shift",
    "edit.insert_silence",
    "edit.separate",
    "edit.separate_on_grid",
    "edit.separate_at_transients",
    "edit.heal",
    "edit.trim_to_selection",
    "edit.trim_start_to_insertion",
    "edit.trim_end_to_insertion",
    "edit.trim_to_file_boundaries",
    "edit.trim_to_fill_selection",
    "edit.mute_clips",
    "edit.copy_to_new_playlist",
    "edit.strip_silence",
    "edit.strip_silence_preview",
    "edit.fades_create",
    "edit.fades_delete",
    "edit.fade_to_start",
    "edit.fade_to_end",
    "edit.nudge",
    "edit.mode",
    "edit.snap_to_grid",
    "edit.shuffle_lock",
    "edit.tool",
    "edit.move_clips",
    "edit.trim_clip",
    "edit.consolidate",
    "edit.cut_pan_automation",
    "edit.cut_plugin_automation",
    "edit.copy_all_automation",
    "edit.copy_pan_automation",
    "edit.copy_plugin_automation",
    "edit.copy_clip_gain",
    "edit.paste_clip_gain",
    "edit.copy_clip_effects",
    "edit.cut_clip_effects",
    "edit.clear_clip_effects",
    "edit.paste_clip_effects",
    "edit.paste_merge_midi",
    "edit.paste_merge_markers",
    "edit.paste_to_current_automation",
    "edit.duplicate_extend",
    "edit.extend_up_to_members",
    "edit.extend_down_to_members",
    "edit.marker_lane_move_up",
    "edit.marker_lane_move_down",
    "edit.marker_lane_extend_up",
    "edit.marker_lane_extend_down",
    "edit.space_clips",
    "edit.snap_next",
    "edit.snap_previous",
    "edit.copy_to_target_playlist",
    "edit.copy_to_duplicate_playlist",
    "edit.move_to_target_playlist",
    "edit.move_to_new_playlist",
    "edit.move_to_duplicate_playlist",
    "edit.tce_to_timeline",
    "edit.automation_coalesce_volume_to_clip_gain",
    "edit.automation_coalesce_clip_gain_to_volume",
    "edit.automation_trim_to_all",
    "edit.automation_glide_to_all",
    "edit.automation_glide_pan",
    // event
    "event.tempo",
    "event.tempo_remove",
    "event.meter",
    "event.tempo_scale",
    "event.tempo_constant",
    "event.insert_time",
    "event.cut_time",
    "event.quantize",
    "event.transpose",
    "event.change_velocity",
    "event.change_duration",
    "event.remove_duplicate_notes",
    "event.add_key_change",
    "event.renumber_bars",
    "event.all_notes_off",
    "event.midi_note",
    // markers
    "markers.add",
    "markers.delete",
    "markers.edit",
    "markers.recall",
    "markers.next",
    "markers.previous",
    // event
    "event.time_ops_window",
    "event.tempo_ops_window",
    "event.midi_ops_window",
    "event.move_song_start",
    "event.tempo_linear",
    "event.tempo_parabolic",
    "event.tempo_s_curve",
    "event.tempo_stretch",
    "event.select_split_notes",
    "event.input_quantize",
    "event.restore_performance",
    "event.flatten_performance",
    "event.midi_track_offsets",
    "event.midi_rtp",
    "event.extract_chords",
    "event.beat_detective",
    "event.identify_beat",
    // session
    "session.audio_health",
    // mix
    "mix.instrument",
    "mix.instrument_param",
    // midi
    "midi.notes",
    "midi.new_clip",
    "midi.note_add",
    "midi.note_edit",
    "midi.note_delete",
    "midi.notes_move",
    // mix
    "mix.volume",
    "mix.pan",
    "mix.mute",
    "mix.solo",
    "mix.solo_safe",
    "mix.clear_solos",
    "mix.clear_mutes",
    "mix.phase_invert",
    "mix.trim",
    "mix.automation_mode",
    "mix.insert",
    "mix.insert_remove",
    "mix.insert_bypass",
    "mix.insert_param",
    "mix.insert_params",
    "mix.insert_move",
    "mix.send",
    "mix.send_remove",
    "mix.send_level",
    "mix.new_bus",
    // automation
    "automation.set_point",
    "automation.write_range",
    "automation.clear",
    "automation.thin",
    "automation.thin_all",
    "automation.write_to_current",
    "automation.write_to_all",
    "automation.volume_to_clip_gain",
    "automation.clip_gain_to_volume",
    "automation.copy_to_send",
    // options
    "options.scrolling",
    "options.solo_mode",
    "options.keyboard_focus",
    "options.pre_roll",
    "options.post_roll",
    // setup
    "setup.session",
    "setup.reset_edit_state",
    // session
    "session.inspect",
    // track
    "track.inspect",
    // clip
    "clip.inspect",
    // engine
    "engine.commands",
    "engine.history",
    "engine.plugins",
    "engine.parity",
    // session
    "session.format_time",
    // mix
    "mix.sampler_clear",
    // setup
    "setup.io",
    "setup.current_timecode",
    "setup.current_feet_frames",
    "setup.external_timecode_offset",
    "setup.video_sync_offset",
    "setup.click_countoff",
    "setup.transcription_settings",
    // mix
    "mix.surround_pan",
    "mix.send_surround_pan",
    // setup
    "setup.main_format",
    // edit
    "edit.tab",
    // track
    "track.new",
    "track.group",
    "track.ungroup",
    "track.group_toggle",
    "track.group_modify",
    "track.duplicate",
    "track.split_into_mono",
    "track.make_inactive",
    "track.move_to_new_folder",
    "track.move_to_folder",
    "track.remove_from_folder",
    "track.change_width",
    "track.delete",
    "track.rename",
    "track.comments",
    "track.color",
    "track.height",
    "track.height_up",
    "track.height_down",
    "track.view",
    "track.hide",
    "track.pin",
    "track.move",
    "track.freeze",
    "track.commit",
    "track.bounce",
    "track.bypass_inserts",
    "track.mute_sends",
    "track.scroll_to",
    "track.clear_clip_indicators",
    "track.clear_trim_automation",
    "track.create_click",
    "track.playlist_new",
    "track.playlist_duplicate",
    "track.playlist_select",
    "track.playlist_delete_unused",
    "track.folder_toggle",
    "track.vca",
    "track.playlist_promote",
    "track.input",
    "track.output",
    "track.timebase",
    "track.elastic",
    "track.object",
    "track.convert_aux_to_folder",
    "track.extract_midi",
    "track.bypass_inserts_ae",
    "track.bypass_inserts_fj",
    "track.bypass_eq",
    "track.bypass_dynamics",
    "track.bypass_reverb",
    "track.bypass_delay",
    "track.bypass_modulation",
    "track.mute_sends_ae",
    "track.mute_sends_fj",
    "track.write_midi_rtp",
    "track.trim_automation_write",
    "track.coalesce_trim",
    "track.coalesce_vca",
    "track.designate_target_playlist",
    "track.show_target_playlist",
    "track.toggle_recent_playlist",
    // transport
    "transport.rtz",
    "transport.go_to_end",
    "transport.locate",
    "transport.status",
    // video
    "video.online",
    "video.offset",
    "video.list",
    // view
    "view.zoom_in",
    "view.zoom_out",
    "view.zoom_by",
    "view.zoom_set",
    "view.zoom_preset",
    "view.zoom_to_selection",
    "view.zoom_fit",
    "view.scroll",
    "view.waveform_zoom",
    "view.ruler",
    "view.main_counter",
    "view.sub_counter",
    "view.grid",
    "view.grid_lines",
    "view.nudge",
    "view.edit_all",
    "view.edit_minimal",
    "view.ruler_timecode2",
    "view.ruler_markers2",
    "view.ruler_markers3",
    "view.ruler_markers4",
    "view.ruler_markers5",
    "view.ruler_all_markers",
    "view.ruler_all",
    "view.ruler_minimal",
    "view.expanded_sends_all",
    "view.expanded_sends_none",
    "view.flags",
    "view.zoom_toggle",
    // mix
    "mix.insert_state",
    // options
    "options.auto_spot_clips",
    "options.automation_follows_edit",
    "options.calibration_mode",
    "options.click",
    "options.countoff",
    "options.delay_compensation",
    "options.destructive_punch",
    "options.dynamic_transport",
    "options.insertion_follows_playback",
    "options.keyboard_lock",
    "options.layered_editing",
    "options.link_timeline_edit",
    "options.link_track_edit",
    "options.loop_playback",
    "options.loop_record",
    "options.low_latency_monitoring",
    "options.marker_target_follows_selection",
    "options.markers_follow_edit",
    "options.midi_live_mode",
    "options.midi_merge",
    "options.midi_thru",
    "options.mirrored_midi",
    "options.pre_fader_metering",
    "options.pre_post_roll",
    "options.quickpunch",
    "options.tab_to_transient",
    "options.transport_online",
    // view
    "view.automation_composite_playlist",
    "view.automation_trim_playlist",
    "view.clip_all_channels",
    "view.clip_ara_note_overlay",
    "view.clip_channel_name",
    "view.clip_copy_move_indicators",
    "view.clip_effects_status",
    "view.clip_gain_info",
    "view.clip_gain_line",
    "view.clip_name",
    "view.clip_overlap_shadows",
    "view.clip_overwrite_indicator",
    "view.clip_processing_state",
    "view.clip_rating",
    "view.clip_scene_take",
    "view.clip_sync_point",
    "view.clip_time_current",
    "view.clip_time_none",
    "view.clip_time_original",
    "view.clip_time_user",
    "view.clip_transparency",
    "view.dropped_video_frame_indicator",
    "view.edit_comments",
    "view.edit_inserts_ae",
    "view.edit_inserts_fj",
    "view.edit_instruments",
    "view.edit_io",
    "view.edit_marker_controls",
    "view.edit_mic_preamps",
    "view.edit_object",
    "view.edit_pinned_tracks",
    "view.edit_realtime_properties",
    "view.edit_sends_ae",
    "view.edit_sends_fj",
    "view.edit_track_color",
    "view.expanded_send_a",
    "view.expanded_send_b",
    "view.expanded_send_c",
    "view.expanded_send_d",
    "view.expanded_send_e",
    "view.expanded_send_f",
    "view.expanded_send_g",
    "view.expanded_send_h",
    "view.expanded_send_i",
    "view.expanded_send_j",
    "view.marker_folder_lines",
    "view.marker_ruler_lane_color",
    "view.marker_ruler_lines",
    "view.marker_track_lane",
    "view.marker_track_lane_color",
    "view.midi_editor_clip_effects",
    "view.midi_input_display",
    "view.ruler_key_staff",
    "view.ruler_tempo_editor",
    "view.track_number",
    "view.track_transcription_lane",
    "view.transport_ableton_link",
    "view.transport_counters",
    "view.transport_expanded",
    "view.transport_midi_controls",
    "view.transport_output_meters",
    "view.transport_synchronization",
    "view.waveform_outlines",
    "view.waveform_overlapped_crossfades",
    "view.waveform_peak",
    "view.waveform_power",
    "view.waveform_rectified",
];

/// Commands the window itself provides.
pub const ARGUMENT_COMMANDS: &[&str] = &[];

/// Registry commands left OUT of the allow-list, grouped by the reason (the audit lists them; every registry command is in exactly one of the two).
pub const LEFT_OUT: &[(&str, &[&str])] = &[
    (
        "files: opens, saves, imports, exports or bounces a file, or loads one by path (files move only through the host; the host saves the opened file itself)",
        &["session.new", "session.open", "session.close", "session.save", "session.save_as", "session.save_copy", "session.save_template", "session.revert", "file.import_audio", "file.import_midi", "file.import_video", "file.bounce_mix", "file.bounce_stems", "file.export_midi", "file.export_session_text", "file.export_clips", "file.export_selected_tracks_as_session", "file.export_sibelius", "file.send_to_sibelius", "file.print_score", "file.score_setup", "file.export_clip_groups", "file.import_clip_groups", "file.get_info", "mix.sampler_load", "file.import_session_data", "track.save_preset", "track.load_preset"],
    ),
    (
        "history: Undo and Redo are the person's",
        &["edit.undo", "edit.redo"],
    ),
    (
        "playback and the audio device: a run edits the session, it does not play it (play, stop, record, pause, wind)",
        &["edit.play_timeline", "transport.play", "transport.stop", "transport.toggle", "transport.record", "transport.pause", "transport.half_speed", "transport.play_selection", "transport.rewind", "transport.fast_forward"],
    ),
    (
        "recording, live input and this machine's audio and MIDI hardware, device and plug-in scanning: the browser build has no device to arm or scan",
        &["engine.clap_plugins", "clip.capture", "event.step_input", "event.retrospective_record", "mix.record_arm", "mix.input_monitor", "options.destructive_record", "options.trackpunch", "setup.preferences", "options.prepare_dpe_tracks", "setup.hardware", "setup.playback_engine", "setup.disk_allocation", "setup.peripherals", "setup.keyboard_shortcuts", "setup.midi_studio", "setup.midi_beat_clock", "setup.midi_input_filter", "setup.midi_input_devices", "track.set_record_to_input_only", "engine.vst3_plugins"],
    ),
    (
        "application commands (quit): not an edit of the session",
        &["app.quit"],
    ),
];

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
