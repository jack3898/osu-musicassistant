# /// script
# requires-python = ">=3.11"
# dependencies = ["py-opensonic==10.4.1"]
# ///
"""Exercise a running server the way Music Assistant's OpenSubsonic provider does, using the same
client library and version MA pins.

Usage: OSU_MA_PASSWORD=secret uv run tools/ma_compat.py [host] [port]
"""
import asyncio, os, sys, time
import aiohttp
from libopensonic import AsyncConnection
from libopensonic.errors import CredentialError, DataNotFoundError, SonicError

PASSWORD = os.environ.get("OSU_MA_PASSWORD", "osu")
BASE = f"http://{sys.argv[1] if len(sys.argv) > 1 else '127.0.0.1'}"
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 4747

def conn(**kw):
    return AsyncConnection(BASE, port=PORT, server_path="", use_get=True, app_name="Music Assistant", **kw)

async def main():
    # Wrong credentials must make ping fail (MA turns this into LoginFailed).
    bad = conn(username="osu", password="wrong")
    try:
        await bad.ping(); raise AssertionError("ping with bad password should fail")
    except CredentialError:
        print("bad password -> CredentialError: ok")
    await bad.cleanup()

    for label, c in [("token", conn(username="osu", password=PASSWORD)),
                     ("legacy", conn(username="osu", password=PASSWORD, legacy_auth=True)),
                     ("apiKey", conn(api_key=PASSWORD))]:
        assert await c.ping(), label
        await c.cleanup()
        print(f"auth {label}: ok")
    c = conn(username="osu", password=PASSWORD)

    ext = await c.get_open_subsonic_extensions()
    print("extensions:", [e.name for e in ext])
    await c.get_podcasts(inc_episodes=False)

    t = time.time()
    artists = await c.get_artists()
    n_artists = sum(len(i.artist or []) for i in artists.index or [])
    print(f"getArtists: {n_artists} artists in {len(artists.index)} index groups ({time.time()-t:.2f}s)")
    first_artist = artists.index[0].artist[0]
    a = await c.get_artist(first_artist.id)
    await c.get_artist_info2(aid=first_artist.id)
    print(f"getArtist: {a.name!r} with {len(a.album or [])} albums")

    t = time.time(); albums = []; offset = 0
    while True:
        page = await c.get_album_list2(ltype="alphabeticalByArtist", size=200, offset=offset)
        if not page: break
        albums += page; offset += 200
    print(f"getAlbumList2: {len(albums)} albums ({time.time()-t:.2f}s)")

    # Library track sync: paginated empty search3, then getAlbum + getAlbumInfo2 + lyrics per album change.
    t = time.time(); songs = []; offset = 0; album_calls = 0; last_album = None
    while True:
        res = await c.search3(query="", artist_count=0, album_count=0, song_offset=offset, song_count=200)
        if not res.song: break
        for s in res.song:
            aid = s.album_id or s.parent
            if aid != last_album:
                await c.get_album(aid); await c.get_album_info2(aid=aid); album_calls += 1; last_album = aid
            await c.get_lyrics_by_song_id(s.id)
        songs += res.song; offset += 200
    print(f"search3 sync: {len(songs)} songs, {album_calls} album lookups ({time.time()-t:.1f}s)")
    missing_dur = [s for s in songs if not s.duration]
    print(f"  songs without duration: {len(missing_dur)}; content types: {sorted({s.content_type for s in songs})}")
    s = songs[len(songs)//2]
    print(f"  sample: {s.artist} - {s.title} | album={s.album!r} dur={s.duration}s {s.content_type} {s.bit_rate}kbps {s.sampling_rate}Hz")

    res = await c.search3(query="camellia", artist_count=5, album_count=5, song_count=5)
    print(f"search 'camellia': {len(res.artist or [])} artists, {len(res.album or [])} albums, {len(res.song or [])} songs")

    pls = await c.get_playlists()
    print(f"getPlaylists: {len(pls)} -> " + ", ".join(f"{p.name} ({p.song_count})" for p in pls[:6]) + ", ...")
    t = time.time()
    allpl = await c.get_playlist(pls[0].id)
    print(f"getPlaylist {allpl.name!r}: {len(allpl.entry)} entries, {allpl.duration//3600}h ({time.time()-t:.2f}s)")
    coll = await c.get_playlist(pls[1].id)
    print(f"getPlaylist {coll.name!r}: first = {coll.entry[0].artist} - {coll.entry[0].title}")

    song = await c.get_song(s.id)
    try:
        await c.get_song("nope"); raise AssertionError("expected not found")
    except DataNotFoundError:
        print("getSong unknown id -> DataNotFoundError: ok")

    art = await c.get_cover_art(song.cover_art)
    body = await art.read()
    print(f"getCoverArt: {art.headers['Content-Type']} {len(body)} bytes")

    url, _ = c.get_stream_url(song.id, tformat="raw", estimate_length=True)
    async with aiohttp.ClientSession() as http:
        async with http.get(url) as r:
            full = await r.read()
            print(f"stream: {r.status} {r.headers['Content-Type']} {len(full)} bytes, accept-ranges={r.headers.get('Accept-Ranges')}")
        async with http.get(url, headers={"Range": "bytes=1000-1999"}) as r:
            part = await r.read()
            assert r.status == 206 and part == full[1000:2000], (r.status, len(part))
            print(f"stream range: {r.status} {r.headers['Content-Range']}")

    for call in [c.get_starred2(), c.get_top_songs("x", 10), c.get_similar_songs(s.id, 10), c.get_internet_radio_stations(), c.get_bookmarks(), c.get_newest_podcasts(10), c.get_album_list2(ltype="newest", size=10), c.get_album_list2(ltype="frequent", size=10)]:
        await call
    print("stubbed endpoints: ok")
    try:
        await c.create_playlist(name="x"); raise AssertionError("expected read-only error")
    except SonicError as e:
        print(f"createPlaylist -> {type(e).__name__}: {e}")
    await c.cleanup()
    print("\nALL CHECKS PASSED")

asyncio.run(main())
