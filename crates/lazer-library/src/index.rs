//! Building a music library (songs and collections) from lazer's database.
//!
//! Lazer stores one `Beatmap` per difficulty, and the difficulties of a set nearly always share one
//! audio file. A [`Song`] is therefore one unique audio file, identified by its SHA-256 hash, which
//! also makes it stable across re-indexes.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::data_dir::DataDir;
use crate::realm::{Class, Object, Property, Realm, RealmError, Value};
use crate::snapshot::Snapshot;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("couldn't snapshot the osu! database: {0}")]
    Snapshot(#[from] std::io::Error),
    #[error(transparent)]
    Realm(#[from] RealmError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Song {
    /// SHA-256 of the audio file; also its name in lazer's file store.
    pub id: String,
    /// The audio file's original name inside the beatmap set, e.g. `audio.mp3`.
    pub audio_filename: String,
    pub title: String,
    pub title_unicode: String,
    pub artist: String,
    pub artist_unicode: String,
    pub source: String,
    pub tags: String,
    /// The beatmap set's creator (mapper).
    pub creator: String,
    /// SHA-256 of the set's background image, used as cover art.
    pub background_id: Option<String>,
    /// lazer's local beatmap set ID.
    pub set_id: String,
    /// osu! website beatmap set ID, if the set is submitted.
    pub online_set_id: Option<i64>,
    /// Longest gameplay length of the difficulties using this audio. Not the audio's real length.
    pub gameplay_length: Duration,
    pub bpm: f64,
    pub date_added: Option<SystemTime>,
    pub date_ranked: Option<SystemTime>,
    pub date_submitted: Option<SystemTime>,
    /// Bundled with lazer rather than imported by the user.
    pub protected: bool,
    /// A sped-up or slowed-down copy of a set's audio made for practice (e.g. by osu-trainer).
    pub rate_edit: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Collection {
    pub id: String,
    pub name: String,
    /// Song IDs in the order the beatmaps were added to the collection, without duplicates.
    pub song_ids: Vec<String>,
    /// Beatmaps in the collection that aren't installed.
    pub missing: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Library {
    /// All songs, ordered by when their beatmap set was imported (oldest first).
    pub songs: Vec<Song>,
    pub collections: Vec<Collection>,
    /// The lazer schema version the library was read from.
    pub schema_version: u64,
    songs_by_id: HashMap<String, usize>,
}

impl Library {
    /// Snapshot the database in `data_dir` and index it.
    pub fn load(data_dir: &DataDir) -> Result<Self, LibraryError> {
        let snapshot = Snapshot::take(&data_dir.database())?;
        let realm = Realm::open_immutable(&snapshot.path())?;
        Ok(read_library(&realm)?)
    }

    pub fn song(&self, id: &str) -> Option<&Song> {
        self.songs_by_id.get(id).map(|&i| &self.songs[i])
    }

    pub fn collection(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id)
    }

    /// What "All songs" plays: everything the user imported, minus practice rate edits.
    pub fn all_songs(&self) -> impl Iterator<Item = &Song> {
        self.songs.iter().filter(|s| !s.protected && !s.rate_edit)
    }
}

/// Property keys for every class we read, looked up once per index.
struct Schema {
    set_class: Class,
    set_id: Property,
    set_online_id: Property,
    set_date_added: Property,
    set_date_ranked: Property,
    set_date_submitted: Property,
    set_beatmaps: Property,
    set_files: Property,
    set_delete_pending: Property,
    set_protected: Property,
    beatmap_md5: Property,
    beatmap_metadata: Property,
    beatmap_length: Property,
    beatmap_bpm: Property,
    meta_title: Property,
    meta_title_unicode: Property,
    meta_artist: Property,
    meta_artist_unicode: Property,
    meta_source: Property,
    meta_tags: Property,
    meta_author: Property,
    meta_audio_file: Property,
    meta_background_file: Property,
    user_username: Property,
    usage_file: Property,
    usage_filename: Property,
    file_hash: Property,
    collection_class: Class,
    collection_id: Property,
    collection_name: Property,
    collection_md5s: Property,
}

impl Schema {
    fn read(realm: &Realm) -> Result<Self, RealmError> {
        let set = realm.class("BeatmapSet")?;
        let beatmap = realm.class("Beatmap")?;
        let meta = realm.class("BeatmapMetadata")?;
        let user = realm.class("RealmUser")?;
        let usage = realm.class("RealmNamedFileUsage")?;
        let file = realm.class("File")?;
        let collection = realm.class("BeatmapCollection")?;
        Ok(Self {
            set_id: set.property("ID")?,
            set_online_id: set.property("OnlineID")?,
            set_date_added: set.property("DateAdded")?,
            set_date_ranked: set.property("DateRanked")?,
            set_date_submitted: set.property("DateSubmitted")?,
            set_beatmaps: set.property("Beatmaps")?,
            set_files: set.property("Files")?,
            set_delete_pending: set.property("DeletePending")?,
            set_protected: set.property("Protected")?,
            set_class: set,
            beatmap_md5: beatmap.property("MD5Hash")?,
            beatmap_metadata: beatmap.property("Metadata")?,
            beatmap_length: beatmap.property("Length")?,
            beatmap_bpm: beatmap.property("BPM")?,
            meta_title: meta.property("Title")?,
            meta_title_unicode: meta.property("TitleUnicode")?,
            meta_artist: meta.property("Artist")?,
            meta_artist_unicode: meta.property("ArtistUnicode")?,
            meta_source: meta.property("Source")?,
            meta_tags: meta.property("Tags")?,
            meta_author: meta.property("Author")?,
            meta_audio_file: meta.property("AudioFile")?,
            meta_background_file: meta.property("BackgroundFile")?,
            user_username: user.property("Username")?,
            usage_file: usage.property("File")?,
            usage_filename: usage.property("Filename")?,
            file_hash: file.property("Hash")?,
            collection_id: collection.property("ID")?,
            collection_name: collection.property("Name")?,
            collection_md5s: collection.property("BeatmapMD5Hashes")?,
            collection_class: collection,
        })
    }
}

fn read_library(realm: &Realm) -> Result<Library, RealmError> {
    let schema = Schema::read(realm)?;
    let mut songs: Vec<Song> = Vec::new();
    let mut songs_by_id: HashMap<String, usize> = HashMap::new();
    let mut song_by_md5: HashMap<String, String> = HashMap::new();

    let sets = realm.all(&schema.set_class)?;
    for set in sets.iter()? {
        let set = set?;
        if bool_of(&set, schema.set_delete_pending, "DeletePending")? {
            continue;
        }
        let files = read_set_files(&set, &schema)?;
        let set_id = uuid_of(&set, schema.set_id, "ID")?.unwrap_or_default();
        let online_set_id =
            Some(int_of(&set, schema.set_online_id, "OnlineID")?).filter(|&id| id > 0);
        let date_added = time_of(&set, schema.set_date_added, "DateAdded")?;
        let date_ranked = time_of(&set, schema.set_date_ranked, "DateRanked")?;
        let date_submitted = time_of(&set, schema.set_date_submitted, "DateSubmitted")?;
        let protected = bool_of(&set, schema.set_protected, "Protected")?;

        let beatmaps = set.list(schema.set_beatmaps)?;
        for i in 0..beatmaps.len()? {
            let beatmap = beatmaps.object(i)?;
            let Some(meta) = beatmap.linked(schema.beatmap_metadata)? else {
                continue;
            };
            let audio_filename = string_of(&meta, schema.meta_audio_file, "AudioFile")?;
            let Some(audio_id) = files.get(&audio_filename.to_lowercase()).cloned() else {
                continue;
            };

            let md5 = string_of(&beatmap, schema.beatmap_md5, "MD5Hash")?;
            if !md5.is_empty() {
                song_by_md5.insert(md5, audio_id.clone());
            }
            let length = Duration::from_secs_f64(
                double_of(&beatmap, schema.beatmap_length, "Length")?.max(0.0) / 1000.0,
            );

            match songs_by_id.entry(audio_id.clone()) {
                Entry::Occupied(e) => {
                    let song = &mut songs[*e.get()];
                    song.gameplay_length = song.gameplay_length.max(length);
                }
                Entry::Vacant(e) => {
                    let creator = match meta.linked(schema.meta_author)? {
                        Some(author) => string_of(&author, schema.user_username, "Username")?,
                        None => String::new(),
                    };
                    let background =
                        string_of(&meta, schema.meta_background_file, "BackgroundFile")?;
                    e.insert(songs.len());
                    songs.push(Song {
                        id: audio_id,
                        rate_edit: is_rate_edit(&audio_filename),
                        audio_filename,
                        title: string_of(&meta, schema.meta_title, "Title")?,
                        title_unicode: string_of(&meta, schema.meta_title_unicode, "TitleUnicode")?,
                        artist: string_of(&meta, schema.meta_artist, "Artist")?,
                        artist_unicode: string_of(
                            &meta,
                            schema.meta_artist_unicode,
                            "ArtistUnicode",
                        )?,
                        source: string_of(&meta, schema.meta_source, "Source")?,
                        tags: string_of(&meta, schema.meta_tags, "Tags")?,
                        creator,
                        background_id: files.get(&background.to_lowercase()).cloned(),
                        set_id: set_id.clone(),
                        online_set_id,
                        gameplay_length: length,
                        bpm: double_of(&beatmap, schema.beatmap_bpm, "BPM")?,
                        date_added,
                        date_ranked,
                        date_submitted,
                        protected,
                    });
                }
            }
        }
    }

    // Order by import date; a stable sort keeps lazer's own order for ties.
    let mut order: Vec<usize> = (0..songs.len()).collect();
    order.sort_by_key(|&i| songs[i].date_added);
    let mut slots: Vec<Option<Song>> = songs.into_iter().map(Some).collect();
    let songs: Vec<Song> = order
        .into_iter()
        .map(|i| slots[i].take().unwrap())
        .collect();
    let songs_by_id = songs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.clone(), i))
        .collect();

    let collections = read_collections(realm, &schema, &song_by_md5)?;
    Ok(Library {
        songs,
        collections,
        schema_version: realm.schema_version(),
        songs_by_id,
    })
}

/// Map each file in a set by its lowercased name to its hash. Lazer matches names case-insensitively.
fn read_set_files(
    set: &Object<'_>,
    schema: &Schema,
) -> Result<HashMap<String, String>, RealmError> {
    let usages = set.list(schema.set_files)?;
    let mut files = HashMap::new();
    for i in 0..usages.len()? {
        let usage = usages.object(i)?;
        let Some(file) = usage.linked(schema.usage_file)? else {
            continue;
        };
        let name = string_of(&usage, schema.usage_filename, "Filename")?;
        files.insert(
            name.to_lowercase(),
            string_of(&file, schema.file_hash, "Hash")?,
        );
    }
    Ok(files)
}

fn read_collections(
    realm: &Realm,
    schema: &Schema,
    song_by_md5: &HashMap<String, String>,
) -> Result<Vec<Collection>, RealmError> {
    let mut collections = Vec::new();
    for collection in realm.all(&schema.collection_class)?.iter()? {
        let collection = collection?;
        let md5s = collection.list(schema.collection_md5s)?;
        let mut song_ids: Vec<String> = Vec::new();
        let mut missing = 0;
        for i in 0..md5s.len()? {
            let Value::String(md5) = md5s.get(i)? else {
                continue;
            };
            match song_by_md5.get(&md5) {
                Some(id) if !song_ids.contains(id) => song_ids.push(id.clone()),
                Some(_) => {}
                None => missing += 1,
            }
        }
        collections.push(Collection {
            id: uuid_of(&collection, schema.collection_id, "ID")?.unwrap_or_default(),
            name: string_of(&collection, schema.collection_name, "Name")?,
            song_ids,
            missing,
        });
    }
    collections.sort_by_key(|c| c.name.to_lowercase());
    Ok(collections)
}

/// Rate-change tools name their output like `audio 1.200x.mp3` or `audio 0.85x (pitch raised).ogg`.
fn is_rate_edit(filename: &str) -> bool {
    let stem = filename
        .rsplit_once('.')
        .map_or(filename, |(stem, _)| stem)
        .trim_end();
    let stem = match stem.strip_suffix(')').and_then(|s| s.rsplit_once(" (")) {
        Some((before, _)) => before,
        None => stem,
    };
    let Some(rate) = stem
        .rsplit(' ')
        .next()
        .and_then(|token| token.strip_suffix('x'))
    else {
        return false;
    };
    stem.contains(' ') && !rate.is_empty() && rate.parse::<f64>().is_ok_and(|r| r > 0.0)
}

fn unexpected(property: &str, expected: &'static str, found: Value) -> RealmError {
    RealmError::UnexpectedType {
        property: property.into(),
        expected,
        found,
    }
}

fn string_of(obj: &Object<'_>, prop: Property, name: &str) -> Result<String, RealmError> {
    match obj.get(prop)? {
        Value::String(s) => Ok(s),
        Value::Null => Ok(String::new()),
        other => Err(unexpected(name, "string", other)),
    }
}

fn int_of(obj: &Object<'_>, prop: Property, name: &str) -> Result<i64, RealmError> {
    match obj.get(prop)? {
        Value::Int(i) => Ok(i),
        other => Err(unexpected(name, "integer", other)),
    }
}

fn bool_of(obj: &Object<'_>, prop: Property, name: &str) -> Result<bool, RealmError> {
    match obj.get(prop)? {
        Value::Bool(b) => Ok(b),
        other => Err(unexpected(name, "bool", other)),
    }
}

fn double_of(obj: &Object<'_>, prop: Property, name: &str) -> Result<f64, RealmError> {
    match obj.get(prop)? {
        Value::Double(d) => Ok(d),
        Value::Float(f) => Ok(f.into()),
        Value::Int(i) => Ok(i as f64),
        other => Err(unexpected(name, "number", other)),
    }
}

fn time_of(obj: &Object<'_>, prop: Property, name: &str) -> Result<Option<SystemTime>, RealmError> {
    match obj.get(prop)? {
        Value::Timestamp { seconds, nanos } => {
            let offset = Duration::new(seconds.unsigned_abs(), nanos.unsigned_abs());
            Ok(if seconds >= 0 {
                UNIX_EPOCH.checked_add(offset)
            } else {
                UNIX_EPOCH.checked_sub(offset)
            })
        }
        Value::Null => Ok(None),
        other => Err(unexpected(name, "timestamp", other)),
    }
}

fn uuid_of(obj: &Object<'_>, prop: Property, name: &str) -> Result<Option<String>, RealmError> {
    match obj.get(prop)? {
        Value::Uuid(b) => Ok(Some(format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        ))),
        Value::Null => Ok(None),
        other => Err(unexpected(name, "uuid", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::is_rate_edit;

    #[test]
    fn detects_rate_edits() {
        assert!(is_rate_edit("audio 1.200x (pitch raised).mp3"));
        assert!(is_rate_edit("audio 0.882x.mp3"));
        assert!(is_rate_edit("audio 1.5x.ogg"));
        assert!(!is_rate_edit("audio.mp3"));
        assert!(!is_rate_edit("mmImpossible Mode 200BPM.mp3"));
        assert!(!is_rate_edit("Nightcore!.mp3"));
        assert!(!is_rate_edit("1.5x.mp3"));
        assert!(!is_rate_edit("remix x.mp3"));
    }
}
