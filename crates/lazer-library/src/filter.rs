//! Hiding songs the user never wants to hear.

use crate::index::Song;

/// Decides which songs are visible anywhere: in "All songs", collections, albums and search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongFilter {
    /// Hide practice rate edits (`audio 1.200x.mp3` and friends).
    pub hide_rate_edits: bool,
    /// Hide songs whose title, artist or audio filename contains any of these (case-insensitive).
    pub blocked_patterns: Vec<String>,
}

impl Default for SongFilter {
    fn default() -> Self {
        Self {
            hide_rate_edits: true,
            blocked_patterns: Vec::new(),
        }
    }
}

impl SongFilter {
    pub fn allows(&self, song: &Song) -> bool {
        if self.hide_rate_edits && song.rate_edit {
            return false;
        }
        let fields = [
            &song.title,
            &song.title_unicode,
            &song.artist,
            &song.artist_unicode,
            &song.audio_filename,
        ];
        !self
            .blocked_patterns
            .iter()
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty())
            .any(|pattern| {
                fields
                    .iter()
                    .any(|field| field.to_lowercase().contains(&pattern))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, filename: &str, rate_edit: bool) -> Song {
        Song {
            title: title.into(),
            audio_filename: filename.into(),
            rate_edit,
            ..Song::default()
        }
    }

    #[test]
    fn hides_rate_edits_by_default() {
        let filter = SongFilter::default();
        assert!(filter.allows(&song("Freedom Dive", "audio.mp3", false)));
        assert!(!filter.allows(&song("Freedom Dive", "audio 1.2x.mp3", true)));
        let filter = SongFilter {
            hide_rate_edits: false,
            ..SongFilter::default()
        };
        assert!(filter.allows(&song("Freedom Dive", "audio 1.2x.mp3", true)));
    }

    #[test]
    fn blocks_patterns_case_insensitively() {
        let filter = SongFilter {
            blocked_patterns: vec!["  nightcore ".into(), String::new()],
            ..SongFilter::default()
        };
        assert!(!filter.allows(&song("Tsukinami (Nightcore Mix)", "audio.mp3", false)));
        assert!(!filter.allows(&song("Song", "NIGHTCORE.mp3", false)));
        assert!(filter.allows(&song("Song", "audio.mp3", false)));
    }
}
