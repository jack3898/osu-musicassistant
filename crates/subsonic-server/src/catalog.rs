//! The library as Subsonic sees it: filtered songs grouped into albums (beatmap sets), artists and
//! playlists (collections plus "All songs").

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use lazer_library::{Library, Song, SongFilter};
use md5::{Digest, Md5};

pub const ALL_SONGS_PLAYLIST_ID: &str = "pl-all";
pub const ALL_SONGS_PLAYLIST_NAME: &str = "osu! — All songs";

/// How the library is presented.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewOptions {
    pub filter: SongFilter,
    /// Show titles and artists in their original script (e.g. Japanese) instead of romanised.
    pub prefer_unicode: bool,
    /// Prepended to collection names so they stand out among other playlists.
    pub collection_prefix: String,
}

#[derive(Debug)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist_id: String,
    /// Indices into [`Library::songs`].
    pub songs: Vec<usize>,
    pub cover_art: Option<String>,
    pub created: Option<SystemTime>,
    pub year: Option<i32>,
}

#[derive(Debug)]
pub struct Artist {
    pub id: String,
    pub name: String,
    /// Indices into [`Catalog::albums`].
    pub albums: Vec<usize>,
}

#[derive(Debug)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    /// The name without [`ViewOptions::collection_prefix`], for UIs that already say what it is.
    pub title: String,
    pub comment: String,
    /// Indices into [`Library::songs`].
    pub songs: Vec<usize>,
    pub created: Option<SystemTime>,
}

#[derive(Debug)]
pub struct Catalog {
    library: Arc<Library>,
    prefer_unicode: bool,
    /// Visible songs in library order, as indices into [`Library::songs`].
    pub songs: Vec<usize>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub playlists: Vec<Playlist>,
    song_by_id: HashMap<String, usize>,
    song_album: HashMap<usize, usize>,
    album_by_id: HashMap<String, usize>,
    artist_by_id: HashMap<String, usize>,
    cover_art: HashSet<String>,
}

impl Catalog {
    pub fn new(library: Arc<Library>, options: &ViewOptions) -> Self {
        let prefer_unicode = options.prefer_unicode;
        let visible = |s: &Song| options.filter.allows(s);
        let songs: Vec<usize> = (0..library.songs.len())
            .filter(|&i| visible(&library.songs[i]))
            .collect();

        let mut albums: Vec<Album> = Vec::new();
        let mut album_by_id = HashMap::new();
        let mut song_album = HashMap::new();
        let mut artists: Vec<Artist> = Vec::new();
        let mut artist_by_id = HashMap::new();
        for &i in &songs {
            let song = &library.songs[i];
            let album_id = format!("al-{}", song.set_id);
            let album_index = *album_by_id.entry(album_id.clone()).or_insert_with(|| {
                let artist_name = or_default(
                    display(&song.artist, &song.artist_unicode, prefer_unicode),
                    UNKNOWN_ARTIST,
                );
                let artist_id = artist_id(&artist_name);
                let artist_index = *artist_by_id.entry(artist_id.clone()).or_insert_with(|| {
                    artists.push(Artist {
                        id: artist_id.clone(),
                        name: artist_name,
                        albums: Vec::new(),
                    });
                    artists.len() - 1
                });
                artists[artist_index].albums.push(albums.len());
                albums.push(Album {
                    id: album_id,
                    name: or_default(
                        display(&song.title, &song.title_unicode, prefer_unicode),
                        UNTITLED,
                    ),
                    artist_id,
                    songs: Vec::new(),
                    cover_art: song.background_id.clone(),
                    created: song.date_added,
                    year: year(song.date_ranked.or(song.date_submitted)),
                });
                albums.len() - 1
            });
            albums[album_index].songs.push(i);
            song_album.insert(i, album_index);
        }

        let song_by_id: HashMap<String, usize> = songs
            .iter()
            .map(|&i| (library.songs[i].id.clone(), i))
            .collect();
        let mut playlists = vec![Playlist {
            id: ALL_SONGS_PLAYLIST_ID.into(),
            name: ALL_SONGS_PLAYLIST_NAME.into(),
            title: "All songs".into(),
            comment: "Every song in your osu! library".into(),
            songs: songs
                .iter()
                .copied()
                .filter(|&i| !library.songs[i].protected)
                .collect(),
            created: songs.first().and_then(|&i| library.songs[i].date_added),
        }];
        for collection in &library.collections {
            playlists.push(Playlist {
                id: format!("pl-{}", collection.id),
                name: format!("{}{}", options.collection_prefix, collection.name),
                title: collection.name.clone(),
                comment: "osu! collection".into(),
                songs: collection
                    .song_ids
                    .iter()
                    .filter_map(|id| song_by_id.get(id).copied())
                    .collect(),
                created: None,
            });
        }

        let cover_art = albums.iter().filter_map(|a| a.cover_art.clone()).collect();
        Self {
            library,
            prefer_unicode,
            songs,
            albums,
            artists,
            playlists,
            song_by_id,
            song_album,
            album_by_id,
            artist_by_id,
            cover_art,
        }
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn song(&self, index: usize) -> &Song {
        &self.library.songs[index]
    }

    /// Look up a visible song by its ID.
    pub fn song_index(&self, id: &str) -> Option<usize> {
        self.song_by_id.get(id).copied()
    }

    pub fn album_of(&self, song_index: usize) -> &Album {
        &self.albums[self.song_album[&song_index]]
    }

    pub fn album(&self, id: &str) -> Option<&Album> {
        self.album_by_id.get(id).map(|&i| &self.albums[i])
    }

    pub fn artist(&self, id: &str) -> Option<&Artist> {
        self.artist_by_id.get(id).map(|&i| &self.artists[i])
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    /// Whether `id` is a background image of a visible album, i.e. safe to serve as cover art.
    pub fn is_cover_art(&self, id: &str) -> bool {
        self.cover_art.contains(id)
    }

    pub fn title(&self, song: &Song) -> String {
        or_default(
            display(&song.title, &song.title_unicode, self.prefer_unicode),
            UNTITLED,
        )
    }
}

const UNKNOWN_ARTIST: &str = "Unknown artist";
const UNTITLED: &str = "Untitled";

fn or_default(value: &str, default: &str) -> String {
    let value = value.trim();
    if value.is_empty() { default } else { value }.to_string()
}

fn display<'a>(romanised: &'a str, unicode: &'a str, prefer_unicode: bool) -> &'a str {
    match (prefer_unicode && !unicode.is_empty()) || romanised.is_empty() {
        true => unicode,
        false => romanised,
    }
}

/// Artists have no ID in lazer, so derive a stable one from the (case-folded) name.
fn artist_id(name: &str) -> String {
    let digest = Md5::digest(name.to_lowercase().as_bytes());
    format!("ar-{}", &hex::encode(digest)[..16])
}

fn year(time: Option<SystemTime>) -> Option<i32> {
    humantime::format_rfc3339(time?)
        .to_string()
        .get(..4)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazer_library::Collection;

    fn song(id: &str, set: &str, artist: &str, rate_edit: bool) -> Song {
        Song {
            id: id.into(),
            set_id: set.into(),
            title: format!("Title {id}"),
            artist: artist.into(),
            audio_filename: "audio.mp3".into(),
            background_id: Some(format!("bg-{set}")),
            rate_edit,
            ..Song::default()
        }
    }

    fn library() -> Arc<Library> {
        let songs = vec![
            song("a", "set1", "Camellia", false),
            song("a-fast", "set1", "Camellia", true),
            song("b", "set2", "camellia", false),
            song("c", "set3", "xi", false),
        ];
        let collections = vec![Collection {
            id: "c1".into(),
            name: "Favourites".into(),
            song_ids: vec!["c".into(), "a-fast".into(), "a".into()],
            missing: 0,
        }];
        Arc::new(Library::new(songs, collections, 52))
    }

    #[test]
    fn groups_filtered_songs() {
        let catalog = Catalog::new(library(), &ViewOptions::default());
        assert_eq!(catalog.songs.len(), 3);
        assert_eq!(catalog.albums.len(), 3);
        // Artist IDs are case-insensitive, so both Camellia spellings share one artist.
        assert_eq!(catalog.artists.len(), 2);
        assert_eq!(catalog.song_index("a-fast"), None);

        let all = catalog.playlist(ALL_SONGS_PLAYLIST_ID).unwrap();
        assert_eq!(all.songs.len(), 3);
        let favourites = catalog.playlist("pl-c1").unwrap();
        let ids: Vec<_> = favourites
            .songs
            .iter()
            .map(|&i| catalog.song(i).id.as_str())
            .collect();
        assert_eq!(ids, ["c", "a"]);
        assert!(catalog.is_cover_art("bg-set1"));
        assert!(!catalog.is_cover_art("audio-hash"));
    }

    #[test]
    fn rate_edits_can_be_shown() {
        let options = ViewOptions {
            filter: SongFilter {
                hide_rate_edits: false,
                ..SongFilter::default()
            },
            ..Default::default()
        };
        let catalog = Catalog::new(library(), &options);
        assert_eq!(catalog.album("al-set1").unwrap().songs.len(), 2);
    }

    #[test]
    fn collections_get_the_prefix() {
        let options = ViewOptions {
            collection_prefix: "osu! collection - ".into(),
            ..Default::default()
        };
        let catalog = Catalog::new(library(), &options);
        let favourites = catalog.playlist("pl-c1").unwrap();
        assert_eq!(favourites.name, "osu! collection - Favourites");
        assert_eq!(favourites.title, "Favourites");
        // "All songs" doubles as the marker the app finds Music Assistant's provider by.
        let all = catalog.playlist(ALL_SONGS_PLAYLIST_ID).unwrap();
        assert_eq!(all.name, ALL_SONGS_PLAYLIST_NAME);
    }
}
