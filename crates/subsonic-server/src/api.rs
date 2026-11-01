//! Subsonic REST endpoints.
//!
//! Clients disagree on URL shape (`/rest/ping.view`, `//ping.view`, `/ping`), so every request goes
//! through one handler that dispatches on the last path segment. Responses are always JSON, which
//! is what Music Assistant's client requests.

// Handlers return a ready-made error response as `Err` for early exits; boxing it buys nothing.
#![allow(clippy::result_large_err)]

use std::hash::{BuildHasher, RandomState};
use std::path::PathBuf;
use std::time::SystemTime;

use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use serde_json::{Map, Value, json};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::Server;
use crate::audio::{AudioCache, content_type_for_extension, sniff_content_type};
use crate::auth::AuthError;
use crate::catalog::{Album, Artist, Catalog, Playlist};

const API_VERSION: &str = "1.16.1";
const SERVER_TYPE: &str = "osu-musicassistant";
const MAX_FORM_BYTES: usize = 64 * 1024;

mod code {
    pub const GENERIC: u32 = 0;
    pub const MISSING_PARAMETER: u32 = 10;
    pub const WRONG_CREDENTIALS: u32 = 40;
    pub const NOT_AUTHORIZED: u32 = 50;
    pub const NOT_FOUND: u32 = 70;
}

/// Query string and (for POST) form parameters.
struct Params(Vec<(String, String)>);

impl Params {
    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn require(&self, name: &str) -> Result<&str, Response> {
        self.get(name).ok_or_else(|| {
            fail(
                code::MISSING_PARAMETER,
                &format!("Required parameter '{name}' is missing"),
            )
        })
    }

    fn usize(&self, name: &str, default: usize, max: usize) -> usize {
        self.get(name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
            .min(max)
    }
}

pub(crate) async fn handle(State(server): State<Server>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let endpoint = parts.uri.path().rsplit('/').next().unwrap_or_default();
    let endpoint = endpoint
        .strip_suffix(".view")
        .unwrap_or(endpoint)
        .to_string();

    let mut pairs: Vec<(String, String)> =
        form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let is_form = parts.headers.get(header::CONTENT_TYPE).is_some_and(|v| {
        v.as_bytes()
            .starts_with(b"application/x-www-form-urlencoded")
    });
    if parts.method == Method::POST && is_form {
        match to_bytes(body, MAX_FORM_BYTES).await {
            Ok(bytes) => pairs.extend(form_urlencoded::parse(&bytes).into_owned()),
            Err(_) => return fail(code::GENERIC, "Request body too large"),
        }
    }
    let params = Params(pairs);
    tracing::debug!(endpoint, "subsonic request");

    if endpoint != "getOpenSubsonicExtensions" {
        match server
            .credentials()
            .verify(|k| params.get(k).map(str::to_owned))
        {
            Ok(()) => {}
            Err(AuthError::Missing) => {
                return fail(
                    code::MISSING_PARAMETER,
                    "Authentication parameters are missing",
                );
            }
            Err(AuthError::Invalid) => {
                return fail(code::WRONG_CREDENTIALS, "Wrong username or password");
            }
        }
    }

    let catalog = server.catalog();
    let request = Request::from_parts(parts, Body::empty());
    let result = match endpoint.as_str() {
        "ping" => Ok(ok(json!({}))),
        "getLicense" => Ok(ok(json!({ "license": { "valid": true } }))),
        "getOpenSubsonicExtensions" => Ok(ok(json!({ "openSubsonicExtensions": [
            { "name": "apiKeyAuthentication", "versions": [1] },
            { "name": "formPost", "versions": [1] },
            { "name": "songLyrics", "versions": [1] },
        ] }))),
        "getMusicFolders" => Ok(ok(
            json!({ "musicFolders": { "musicFolder": [{ "id": 1, "name": "osu!" }] } }),
        )),
        "getScanStatus" => Ok(ok(
            json!({ "scanStatus": { "scanning": false, "count": catalog.songs.len() } }),
        )),
        "getUser" => Ok(get_user(&server)),
        "getArtists" => Ok(get_artists(&catalog)),
        "getArtist" => get_artist(&server, &catalog, &params),
        "getAlbum" => get_album(&server, &catalog, &params),
        "getSong" => get_song(&server, &catalog, &params),
        "getAlbumList2" => get_album_list2(&server, &catalog, &params),
        "getRandomSongs" => Ok(get_random_songs(&server, &catalog, &params)),
        "search3" => Ok(search3(&server, &catalog, &params)),
        "getPlaylists" => Ok(get_playlists(&server, &catalog)),
        "getPlaylist" => get_playlist(&server, &catalog, &params),
        "stream" | "download" => stream(&server, &catalog, &params, request).await,
        "getCoverArt" => cover_art(&server, &catalog, &params, request).await,
        // Features we have no data for: answer with empty results so clients carry on.
        "getArtistInfo" => Ok(ok(json!({ "artistInfo": {} }))),
        "getArtistInfo2" => Ok(ok(json!({ "artistInfo2": {} }))),
        "getAlbumInfo" | "getAlbumInfo2" => Ok(ok(json!({ "albumInfo": {} }))),
        "getGenres" => Ok(ok(json!({ "genres": {} }))),
        "getStarred" => Ok(ok(json!({ "starred": {} }))),
        "getStarred2" => Ok(ok(json!({ "starred2": {} }))),
        "getTopSongs" => Ok(ok(json!({ "topSongs": {} }))),
        "getSimilarSongs" => Ok(ok(json!({ "similarSongs": {} }))),
        "getSimilarSongs2" => Ok(ok(json!({ "similarSongs2": {} }))),
        "getLyrics" => Ok(ok(json!({ "lyrics": {} }))),
        "getLyricsBySongId" => Ok(ok(json!({ "lyricsList": {} }))),
        "getInternetRadioStations" => Ok(ok(json!({ "internetRadioStations": {} }))),
        "getPodcasts" => Ok(ok(json!({ "podcasts": {} }))),
        "getNewestPodcasts" => Ok(ok(json!({ "newestPodcasts": {} }))),
        "getBookmarks" => Ok(ok(json!({ "bookmarks": {} }))),
        "scrobble" => Ok(ok(json!({}))),
        // The library belongs to osu!; it can't be edited from here.
        "star" | "unstar" | "setRating" | "createPlaylist" | "updatePlaylist"
        | "deletePlaylist" | "createBookmark" | "deleteBookmark" | "savePlayQueue"
        | "startScan" => Err(fail(code::NOT_AUTHORIZED, "This server is read-only")),
        _ => Err(fail(
            code::GENERIC,
            &format!("Endpoint '{endpoint}' is not supported"),
        )),
    };
    result.unwrap_or_else(|response| response)
}

fn envelope(status: &str, payload: Value) -> Response {
    let mut body = json!({
        "status": status,
        "version": API_VERSION,
        "type": SERVER_TYPE,
        "serverVersion": env!("CARGO_PKG_VERSION"),
        "openSubsonic": true,
    });
    if let (Value::Object(body), Value::Object(payload)) = (&mut body, payload) {
        body.extend(payload);
    }
    Json(json!({ "subsonic-response": body })).into_response()
}

fn ok(payload: Value) -> Response {
    envelope("ok", payload)
}

fn fail(code: u32, message: &str) -> Response {
    envelope(
        "failed",
        json!({ "error": { "code": code, "message": message } }),
    )
}

fn not_found(what: &str) -> Response {
    fail(code::NOT_FOUND, &format!("{what} not found"))
}

/// Remove `null` fields so optional values are simply absent, as Subsonic clients expect.
fn compact(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .collect::<Map<String, Value>>(),
        ),
        other => other,
    }
}

fn timestamp(time: Option<SystemTime>) -> String {
    humantime::format_rfc3339_seconds(time.unwrap_or(SystemTime::UNIX_EPOCH)).to_string()
}

fn duration_secs(catalog: &Catalog, audio: &AudioCache, song_index: usize) -> u64 {
    let song = catalog.song(song_index);
    let ms = audio
        .get(&song.id)
        .map_or(song.gameplay_length.as_millis() as u64, |a| a.duration_ms);
    (ms + 500) / 1000
}

fn song_json(catalog: &Catalog, audio: &AudioCache, index: usize) -> Value {
    let song = catalog.song(index);
    let album = catalog.album_of(index);
    let artist = catalog
        .artist(&album.artist_id)
        .map_or("", |a| a.name.as_str());
    let title = catalog.title(song);
    let info = audio.get(&song.id);
    let (content_type, suffix) = match info {
        Some(info) => (info.content_type.clone(), info.suffix.clone()),
        None => {
            let (mime, ext) = content_type_for_extension(&song.audio_filename);
            (mime.to_string(), ext)
        }
    };
    compact(json!({
        "id": song.id,
        "parent": album.id,
        "isDir": false,
        "title": title,
        "album": album.name,
        "albumId": album.id,
        "artist": artist,
        "artistId": album.artist_id,
        "artists": [{ "id": album.artist_id, "name": artist }],
        "displayArtist": artist,
        "coverArt": song.background_id,
        "duration": duration_secs(catalog, audio, index),
        "size": info.map(|i| i.size),
        "contentType": content_type,
        "suffix": suffix,
        "bitRate": info.and_then(|i| i.bitrate_kbps),
        "samplingRate": info.and_then(|i| i.sample_rate),
        "channelCount": info.and_then(|i| i.channels),
        "bpm": (song.bpm > 0.0).then(|| song.bpm.round() as u32),
        "year": album.year,
        "created": timestamp(song.date_added),
        "path": format!("{artist}/{title}.{suffix}").replace(['\\', '\0'], "_"),
        "type": "music",
        "mediaType": "song",
        "isVideo": false,
    }))
}

fn album_json(catalog: &Catalog, audio: &AudioCache, album: &Album, with_songs: bool) -> Value {
    let artist = catalog
        .artist(&album.artist_id)
        .map_or("", |a| a.name.as_str());
    let duration: u64 = album
        .songs
        .iter()
        .map(|&i| duration_secs(catalog, audio, i))
        .sum();
    let mut json = compact(json!({
        "id": album.id,
        "name": album.name,
        "artist": artist,
        "artistId": album.artist_id,
        "artists": [{ "id": album.artist_id, "name": artist }],
        "displayArtist": artist,
        "coverArt": album.cover_art,
        "songCount": album.songs.len(),
        "duration": duration,
        "created": timestamp(album.created),
        "year": album.year,
        "isCompilation": false,
    }));
    if with_songs {
        json["song"] = album
            .songs
            .iter()
            .map(|&i| song_json(catalog, audio, i))
            .collect();
    }
    json
}

fn artist_json(catalog: &Catalog, artist: &Artist) -> Value {
    compact(json!({
        "id": artist.id,
        "name": artist.name,
        "albumCount": artist.albums.len(),
        "coverArt": artist.albums.iter().find_map(|&i| catalog.albums[i].cover_art.clone()),
    }))
}

fn playlist_json(
    catalog: &Catalog,
    audio: &AudioCache,
    playlist: &Playlist,
    with_songs: bool,
) -> Value {
    let duration: u64 = playlist
        .songs
        .iter()
        .map(|&i| duration_secs(catalog, audio, i))
        .sum();
    let created = timestamp(playlist.created);
    let mut json = compact(json!({
        "id": playlist.id,
        "name": playlist.name,
        "comment": playlist.comment,
        "owner": "osu!",
        "public": false,
        "readonly": true,
        "songCount": playlist.songs.len(),
        "duration": duration,
        "created": created,
        "changed": created,
        "coverArt": playlist.songs.iter().find_map(|&i| catalog.song(i).background_id.clone()),
    }));
    if with_songs {
        json["entry"] = playlist
            .songs
            .iter()
            .map(|&i| song_json(catalog, audio, i))
            .collect();
    }
    json
}

fn get_user(server: &Server) -> Response {
    ok(json!({ "user": {
        "username": server.credentials().username,
        "scrobblingEnabled": false,
        "adminRole": false, "settingsRole": false, "downloadRole": true, "uploadRole": false,
        "playlistRole": false, "coverArtRole": false, "commentRole": false, "podcastRole": false,
        "streamRole": true, "jukeboxRole": false, "shareRole": false, "videoConversionRole": false,
        "folder": [1],
    } }))
}

fn get_artists(catalog: &Catalog) -> Response {
    let mut artists: Vec<&Artist> = catalog.artists.iter().collect();
    artists.sort_by_cached_key(|a| a.name.to_lowercase());
    let mut index: Vec<(String, Vec<Value>)> = Vec::new();
    for artist in artists {
        let letter = artist
            .name
            .chars()
            .next()
            .filter(|c| c.is_alphabetic())
            .map_or("#".to_string(), |c| c.to_uppercase().to_string());
        match index.last_mut() {
            Some((name, entries)) if *name == letter => entries.push(artist_json(catalog, artist)),
            _ => index.push((letter, vec![artist_json(catalog, artist)])),
        }
    }
    let index: Vec<Value> = index
        .into_iter()
        .map(|(name, artist)| json!({ "name": name, "artist": artist }))
        .collect();
    ok(json!({ "artists": { "ignoredArticles": "", "index": index } }))
}

fn get_artist(server: &Server, catalog: &Catalog, params: &Params) -> Result<Response, Response> {
    let artist = catalog
        .artist(params.require("id")?)
        .ok_or_else(|| not_found("Artist"))?;
    let audio = server.audio();
    let mut json = artist_json(catalog, artist);
    json["album"] = artist
        .albums
        .iter()
        .map(|&i| album_json(catalog, &audio, &catalog.albums[i], false))
        .collect();
    Ok(ok(json!({ "artist": json })))
}

fn get_album(server: &Server, catalog: &Catalog, params: &Params) -> Result<Response, Response> {
    let album = catalog
        .album(params.require("id")?)
        .ok_or_else(|| not_found("Album"))?;
    Ok(ok(
        json!({ "album": album_json(catalog, &server.audio(), album, true) }),
    ))
}

fn get_song(server: &Server, catalog: &Catalog, params: &Params) -> Result<Response, Response> {
    let index = catalog
        .song_index(params.require("id")?)
        .ok_or_else(|| not_found("Song"))?;
    Ok(ok(
        json!({ "song": song_json(catalog, &server.audio(), index) }),
    ))
}

fn get_album_list2(
    server: &Server,
    catalog: &Catalog,
    params: &Params,
) -> Result<Response, Response> {
    let list_type = params.require("type")?;
    let size = params.usize("size", 10, 500);
    let offset = params.usize("offset", 0, usize::MAX);
    let mut albums: Vec<&Album> = catalog.albums.iter().collect();
    match list_type {
        "alphabeticalByName" => albums.sort_by_cached_key(|a| a.name.to_lowercase()),
        "alphabeticalByArtist" => albums.sort_by_cached_key(|a| {
            let artist = catalog
                .artist(&a.artist_id)
                .map(|a| a.name.to_lowercase())
                .unwrap_or_default();
            (artist, a.name.to_lowercase())
        }),
        "newest" => albums.sort_by_key(|a| std::cmp::Reverse(a.created)),
        "random" => albums = shuffled(&albums),
        "byYear" => {
            let from: i32 = params
                .get("fromYear")
                .and_then(|v| v.parse().ok())
                .unwrap_or(i32::MIN);
            let to: i32 = params
                .get("toYear")
                .and_then(|v| v.parse().ok())
                .unwrap_or(i32::MAX);
            let (low, high) = (from.min(to), from.max(to));
            albums.retain(|a| a.year.is_some_and(|y| (low..=high).contains(&y)));
            albums.sort_by_key(|a| {
                if from <= to {
                    a.year
                } else {
                    a.year.map(|y| -y)
                }
            });
        }
        // No play counts, ratings, stars or genres in osu!'s data.
        _ => albums.clear(),
    }
    let audio = server.audio();
    let page: Vec<Value> = albums
        .into_iter()
        .skip(offset)
        .take(size)
        .map(|a| album_json(catalog, &audio, a, false))
        .collect();
    Ok(ok(json!({ "albumList2": { "album": page } })))
}

fn get_random_songs(server: &Server, catalog: &Catalog, params: &Params) -> Response {
    let size = params.usize("size", 10, 500);
    let audio = server.audio();
    let songs: Vec<Value> = shuffled(&catalog.songs)
        .into_iter()
        .take(size)
        .map(|i| song_json(catalog, &audio, i))
        .collect();
    ok(json!({ "randomSongs": { "song": songs } }))
}

fn search3(server: &Server, catalog: &Catalog, params: &Params) -> Response {
    // Older clients send `""` to mean "everything".
    let query = params
        .get("query")
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .trim()
        .to_lowercase();
    let matches = |fields: &[&str]| {
        query.is_empty() || fields.iter().any(|f| f.to_lowercase().contains(&query))
    };
    let page = |prefix: &str, default: usize| {
        (
            params.usize(&format!("{prefix}Count"), default, 500),
            params.usize(&format!("{prefix}Offset"), 0, usize::MAX),
        )
    };
    let audio = server.audio();

    let (count, offset) = page("artist", 20);
    let artists: Vec<Value> = catalog
        .artists
        .iter()
        .filter(|a| matches(&[&a.name]))
        .skip(offset)
        .take(count)
        .map(|a| artist_json(catalog, a))
        .collect();
    let (count, offset) = page("album", 20);
    let albums: Vec<Value> = catalog
        .albums
        .iter()
        .filter(|a| {
            let artist = catalog.artist(&a.artist_id).map_or("", |a| a.name.as_str());
            matches(&[&a.name, artist])
        })
        .skip(offset)
        .take(count)
        .map(|a| album_json(catalog, &audio, a, false))
        .collect();
    let (count, offset) = page("song", 20);
    let songs: Vec<Value> = catalog
        .songs
        .iter()
        .copied()
        .filter(|&i| {
            let s = catalog.song(i);
            matches(&[
                &s.title,
                &s.title_unicode,
                &s.artist,
                &s.artist_unicode,
                &s.source,
                &s.creator,
            ])
        })
        .skip(offset)
        .take(count)
        .map(|i| song_json(catalog, &audio, i))
        .collect();
    ok(json!({ "searchResult3": { "artist": artists, "album": albums, "song": songs } }))
}

fn get_playlists(server: &Server, catalog: &Catalog) -> Response {
    let audio = server.audio();
    let playlists: Vec<Value> = catalog
        .playlists
        .iter()
        .map(|p| playlist_json(catalog, &audio, p, false))
        .collect();
    ok(json!({ "playlists": { "playlist": playlists } }))
}

fn get_playlist(server: &Server, catalog: &Catalog, params: &Params) -> Result<Response, Response> {
    let playlist = catalog
        .playlist(params.require("id")?)
        .ok_or_else(|| not_found("Playlist"))?;
    Ok(ok(
        json!({ "playlist": playlist_json(catalog, &server.audio(), playlist, true) }),
    ))
}

async fn stream(
    server: &Server,
    catalog: &Catalog,
    params: &Params,
    request: Request,
) -> Result<Response, Response> {
    let index = catalog
        .song_index(params.require("id")?)
        .ok_or_else(|| not_found("Song"))?;
    let song = catalog.song(index);
    let path = server
        .data_dir()
        .file_path(&song.id)
        .ok_or_else(|| not_found("Song"))?;
    let probed = server.audio().get(&song.id).map(|a| a.content_type.clone());
    serve_file(path, probed, request).await
}

async fn cover_art(
    server: &Server,
    catalog: &Catalog,
    params: &Params,
    request: Request,
) -> Result<Response, Response> {
    let id = params.require("id")?;
    let id = match catalog.album(id) {
        Some(album) => album
            .cover_art
            .as_deref()
            .ok_or_else(|| not_found("Cover art"))?,
        None => id,
    };
    if !catalog.is_cover_art(id) {
        return Err(not_found("Cover art"));
    }
    let path = server
        .data_dir()
        .file_path(id)
        .ok_or_else(|| not_found("Cover art"))?;
    serve_file(path, None, request).await
}

/// Serve a file from lazer's store with HTTP Range support. Stored files have no extension, so
/// the content type comes from probing or the file's magic bytes.
async fn serve_file(
    path: PathBuf,
    content_type: Option<String>,
    request: Request,
) -> Result<Response, Response> {
    let content_type = match content_type {
        Some(ct) => ct,
        None => {
            let sniff_path = path.clone();
            tokio::task::spawn_blocking(move || sniff_content_type(&sniff_path))
                .await
                .map_err(|_| fail(code::GENERIC, "Internal error"))?
                .map_err(|_| not_found("File"))?
                .to_string()
        }
    };
    let mime = content_type
        .parse()
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);
    match ServeFile::new_with_mime(path, &mime).oneshot(request).await {
        Ok(response) if response.status() == StatusCode::NOT_FOUND => Err(not_found("File")),
        Ok(response) => Ok(response.map(Body::new)),
        Err(_) => Err(fail(code::GENERIC, "Couldn't read file")),
    }
}

fn shuffled<T: Copy>(items: &[T]) -> Vec<T> {
    let state = RandomState::new();
    let mut keyed: Vec<(u64, T)> = items
        .iter()
        .enumerate()
        .map(|(i, &item)| (state.hash_one(i), item))
        .collect();
    keyed.sort_unstable_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, item)| item).collect()
}
