#!/usr/bin/env python3
"""Crawl a YouTube playlist → timed transcript segments for every video (no API key).

Reads a playlist URL, enumerates its videos with yt-dlp (--flat-playlist, no
download), then fetches each transcript with youtube-transcript-api. Prints a
JSON array to stdout:

    [{"videoId": "...", "title": "...", "segments": [{"start":0.0,"end":3.0,"text":"..."}]}]

The segment shape matches what POST /api/rag/ingest accepts, so the sidecar can
save each video directly. Videos without a transcript are skipped (warned on
stderr) — the crawl still succeeds with the ones that do.

Dependencies (installed into the app's Python venv): yt-dlp, youtube-transcript-api.
Both fetch public data only; no API key, no account.
"""
import json
import subprocess
import sys


def enumerate_playlist(url: str) -> list[tuple[str, str]]:
    """Return [(videoId, title)] for a playlist without downloading anything."""
    proc = subprocess.run(
        [sys.executable, "-m", "yt_dlp", "--flat-playlist", "--print", "%(id)s\t%(title)s", url],
        capture_output=True, text=True, timeout=600,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"yt-dlp failed: {proc.stderr.strip()[-500:]}")
    items = []
    for line in proc.stdout.splitlines():
        if "\t" in line:
            vid, title = line.split("\t", 1)
            items.append((vid, title))
    if not items:
        raise RuntimeError("yt-dlp returned no videos (bad playlist URL?)")
    return items


def transcript(video_id: str) -> list[dict]:
    """Return [{start, end, text}] for one video, in ANY language.

    Uses .list() so we pick whatever transcript exists (auto-generated, any
    language) instead of the default English-only fetch — playlists are often
    in other languages. Raises when none exists or the IP is blocked.
    """
    from youtube_transcript_api import YouTubeTranscriptApi
    api = YouTubeTranscriptApi()
    try:
        transcript_list = api.list(video_id)          # v1.x: any language
        snippets = []
        for available in transcript_list:
            snippets = list(available.fetch())
            if snippets:
                break
    except AttributeError:
        # <1.0: fall back to a broad language list
        snippets = YouTubeTranscriptApi.get_transcript(
            video_id,
            languages=["en", "es", "vi", "pt", "fr", "de", "ja", "ko", "zh", "hi"],
        )
    if not snippets:
        raise RuntimeError("no transcript")
    segments = []
    for s in snippets:
        if isinstance(s, dict):
            start, duration, text = s.get("start", 0), s.get("duration", 0), s.get("text", "")
        else:
            start, duration, text = s.start, s.duration, s.text
        segments.append({
            "start": round(float(start), 2),
            "end": round(float(start) + float(duration), 2),
            "text": text,
        })
    return segments


def main() -> int:
    args = sys.argv[1:]
    limit = 0
    if args and args[0] == "--limit":
        limit = int(args[1]); args = args[2:]
    if len(args) != 1:
        print("usage: yt-crawler.py [--limit N] <playlist_url>", file=sys.stderr)
        return 2
    from youtube_transcript_api import IpBlocked, RequestBlocked
    results = []
    videos = enumerate_playlist(args[0])
    if limit:
        videos = videos[:limit]
    for video_id, title in videos:
        try:
            segments = transcript(video_id)
        except (IpBlocked, RequestBlocked) as error:
            # YouTube is rate-limiting this IP: stop instead of hammering it —
            # return what we have so far and let the caller report partial success.
            print(f"blocked after {len(results)} videos: {error}", file=sys.stderr)
            break
        except Exception as error:  # no transcript / private / unavailable
            print(f"skip {video_id}: {error}", file=sys.stderr)
            continue
        if segments:
            results.append({"videoId": video_id, "title": title, "segments": segments})
    json.dump(results, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
