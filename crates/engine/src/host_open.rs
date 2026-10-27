//! Opening an audio FILE as the host's document (OneOS `craft:open`): the file becomes a session of one track holding one clip, at the
//! file's own sample rate, and Save writes the session's main mix back as the SAME format and bit depth under the file's own name (a
//! new version of the file). A session file (`.scraftz`) is the editor's own document and opens through [`crate::io::open_session_bytes`].
//!
//! Only the formats the editor can WRITE are opened as a document (WAV, AIFF and FLAC): an MP3 or an Ogg file decodes, but saving it
//! back would silently turn it into another format, so the host does not offer to edit it.

use soundcraft_audio_io::{FileFormat, SampleFormat};
use soundcraft_model::{BitDepthSetting, Session};
use soundcraft_time::SampleRate;

use crate::{Engine, EngineError, Result};

/// What an opened audio file is saved back as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFileFormat {
    /// `wav`, `aiff` or `flac`: `file.bounce_mix {format}`.
    pub format: &'static str,
    /// `16`, `24` or `32f`: `file.bounce_mix {bit_depth}`.
    pub bit_depth: &'static str,
    /// The file's sample rate, which the session takes.
    pub rate: u32,
}

/// Replace the engine's session with one holding the audio file `name` (`bytes`), and report how it is written back.
pub fn open_audio_file(e: &mut Engine, name: &str, bytes: &[u8]) -> Result<AudioFileFormat> {
    let ext = std::path::Path::new(name).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
    let info = soundcraft_audio_io::probe(bytes, ext.as_deref()).map_err(|err| EngineError::Io(format!("{name}: {err}")))?;
    let format = match info.format {
        FileFormat::Wav => "wav",
        FileFormat::Aiff => "aiff",
        FileFormat::Flac => "flac",
        other => {
            return Err(EngineError::Io(format!(
                "{name}: Sound writes WAV, AIFF and FLAC, so a {other:?} file would be converted by a save -- it is not opened as a document"
            )));
        }
    };
    let (bit_depth, setting) = match info.sample_format {
        SampleFormat::Int8 | SampleFormat::Int16 => ("16", BitDepthSetting::Int16),
        SampleFormat::Int24 => ("24", BitDepthSetting::Int24),
        SampleFormat::Int32 | SampleFormat::Float32 | SampleFormat::Float64 | SampleFormat::Compressed => ("32f", BitDepthSetting::Float32),
    };
    let rate = SampleRate::new(info.sample_rate).map_err(|err| EngineError::Io(format!("{name}: {err}")))?;
    let stem = std::path::Path::new(name).file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Audio").to_string();
    let mut session = Session::new(stem, rate);
    session.bit_depth = setting;
    e.replace_session(session);
    e.path = None;
    crate::io::import_audio_bytes(e, name, bytes, None, None, 0)?;
    e.mark_clean();
    Ok(AudioFileFormat { format, bit_depth, rate: info.sample_rate })
}
