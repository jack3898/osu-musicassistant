//! Real audio properties (duration, format, bitrate) for songs.
//!
//! Lazer only knows gameplay length, which often stops well before the song does, and osu! audio
//! files sometimes have the wrong extension. Each file is probed once; because files are named by
//! their content hash, results never go stale and can be cached on disk forever.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::probe::Probe;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioInfo {
    pub duration_ms: u64,
    pub size: u64,
    pub content_type: String,
    pub suffix: String,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
}

impl AudioInfo {
    pub fn probe(path: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let size = fs::metadata(path)?.len();
        let tagged = Probe::open(path)?
            .options(ParseOptions::new().read_tags(false).read_cover_art(false))
            .guess_file_type()?
            .read()?;
        let props = tagged.properties();
        let (content_type, suffix) = format_of(tagged.file_type());
        Ok(Self {
            duration_ms: props.duration().as_millis() as u64,
            size,
            content_type: content_type.into(),
            suffix: suffix.into(),
            bitrate_kbps: props.audio_bitrate(),
            sample_rate: props.sample_rate(),
            channels: props.channels(),
        })
    }
}

fn format_of(file_type: FileType) -> (&'static str, &'static str) {
    match file_type {
        FileType::Mpeg => ("audio/mpeg", "mp3"),
        FileType::Vorbis => ("audio/ogg", "ogg"),
        FileType::Opus => ("audio/ogg", "opus"),
        FileType::Flac => ("audio/flac", "flac"),
        FileType::Wav => ("audio/wav", "wav"),
        FileType::Mp4 => ("audio/mp4", "m4a"),
        FileType::Aac => ("audio/aac", "aac"),
        FileType::Aiff => ("audio/aiff", "aiff"),
        _ => ("application/octet-stream", "bin"),
    }
}

/// Guess a MIME type from a filename's extension, for files that haven't been probed yet.
pub fn content_type_for_extension(filename: &str) -> (&'static str, String) {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m4a" | "mp4" => "audio/mp4",
        _ => "application/octet-stream",
    };
    (mime, ext)
}

/// Identify a file's type from its first bytes. Used to serve files whose real type may not
/// match their extension.
pub fn sniff_content_type(path: &Path) -> io::Result<&'static str> {
    let mut header = [0u8; 12];
    let n = File::open(path)?.read(&mut header)?;
    let header = &header[..n];
    Ok(match header {
        [b'I', b'D', b'3', ..] => "audio/mpeg",
        [0xFF, b, ..] if b & 0xE0 == 0xE0 => "audio/mpeg",
        [b'O', b'g', b'g', b'S', ..] => "audio/ogg",
        [b'f', b'L', b'a', b'C', ..] => "audio/flac",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E'] => "audio/wav",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P'] => "image/webp",
        [b'B', b'M', ..] => "image/bmp",
        _ => "application/octet-stream",
    })
}

/// Probed audio info by song ID, persisted as JSON between runs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AudioCache {
    entries: HashMap<String, AudioInfo>,
}

impl AudioCache {
    /// Load a cache file; a missing or unreadable cache just starts empty.
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn get(&self, id: &str) -> Option<&AudioInfo> {
        self.entries.get(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    pub fn insert(&mut self, id: String, info: AudioInfo) {
        self.entries.insert(id, info);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_magic_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(&[u8], &str); 4] = [
            (b"ID3\x04\0\0\0\0", "audio/mpeg"),
            (b"OggS\0\x02", "audio/ogg"),
            (b"RIFF\0\0\0\0WAVEfmt ", "audio/wav"),
            (b"\xFF\xD8\xFF\xE0", "image/jpeg"),
        ];
        for (i, (bytes, expected)) in cases.into_iter().enumerate() {
            let path = dir.path().join(i.to_string());
            fs::write(&path, bytes).unwrap();
            assert_eq!(sniff_content_type(&path).unwrap(), expected);
        }
    }

    #[test]
    fn guesses_from_extension() {
        assert_eq!(
            content_type_for_extension("audio.MP3"),
            ("audio/mpeg", "mp3".into())
        );
        assert_eq!(
            content_type_for_extension("noext").0,
            "application/octet-stream"
        );
    }
}
