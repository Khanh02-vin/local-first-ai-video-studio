#!/usr/bin/env python3
"""Fetch YouTube transcripts (no API key) — one video or a whole playlist.

Two modes:

    yt-crawler.py --video <url>       one object on stdout:
        {"videoId": "...", "title": "...", "duration": 900.0,
         "language": "en", "segments": [{"start":0.0,"end":3.0,"text":"..."}]}

    yt-crawler.py [--limit N] <playlist_url>    a JSON array on stdout:
        [{"videoId": "...", "title": "...", "segments": [...]}]

Metadata comes from yt-dlp (--skip-download, no video bytes) and transcripts
from youtube-transcript-api — both fetch public text only, in any language
(auto-generated included). Playlist shape matches POST /api/rag/ingest; videos
without a transcript are skipped (warned on stderr). The --video mode fails
loudly instead: callers need that one transcript.

Dependencies (installed into the app's Python venv): yt-dlp, youtube-transcript-api.
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


def video_info(url: str) -> tuple[str, str, float]:
    """Return (videoId, title, duration) for a single video — metadata only."""
    proc = subprocess.run(
        [sys.executable, "-m", "yt_dlp", "--skip-download", "--no-playlist",
         "--print", "%(id)s\t%(title)s\t%(duration)s", url],
        capture_output=True, text=True, timeout=120,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"yt-dlp failed: {proc.stderr.strip()[-500:]}")
    lines = proc.stdout.splitlines()
    if not lines or "\t" not in lines[0]:
        raise RuntimeError("yt-dlp returned no metadata (bad video URL?)")
    video_id, title, duration_text = lines[0].split("\t", 2)
    try:
        duration = float(duration_text)
    except ValueError:
        duration = 0.0
    if not video_id or duration <= 0:
        raise RuntimeError(f"yt-dlp returned no usable duration ({duration_text!r})")
    return video_id, title, duration


def transcript(video_id: str) -> tuple[list[dict], str]:
    """Return ([{start, end, text}], language) for one video, in ANY language.

    Uses .list() so we pick whatever transcript exists (auto-generated, any
    language) instead of the default English-only fetch — videos are often in
    other languages. Raises when none exists or the IP is blocked.
    """
    from youtube_transcript_api import YouTubeTranscriptApi
    api = YouTubeTranscriptApi()
    language = ""
    try:
        transcript_list = api.list(video_id)          # v1.x: any language
        snippets = []
        for available in transcript_list:
            snippets = list(available.fetch())
            if snippets:
                language = getattr(available, "language_code", "") or ""
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
    return segments, (language or "und")


def crawl_video(url: str) -> int:
    """One video → transcript JSON on stdout. Fails loudly (exit 1) on any error."""
    from youtube_transcript_api import IpBlocked, RequestBlocked
    video_id, title, duration = video_info(url)
    try:
        segments, language = transcript(video_id)
    except (IpBlocked, RequestBlocked) as error:
        print(f"transcript blocked for {video_id}: {error}", file=sys.stderr)
        return 1
    except Exception as error:  # no transcript / private / unavailable
        print(f"no transcript for {video_id}: {error}", file=sys.stderr)
        return 1
    json.dump({"videoId": video_id, "title": title, "duration": duration,
               "language": language, "segments": segments}, sys.stdout)
    return 0


def main() -> int:
    args = sys.argv[1:]
    if args[:1] == ["--video"]:
        if len(args) != 2:
            print("usage: yt-crawler.py --video <url>", file=sys.stderr)
            return 2
        return crawl_video(args[1])
    limit = 0
    if args and args[0] == "--limit":
        limit = int(args[1]); args = args[2:]
    if len(args) != 1:
        print("usage: yt-crawler.py [--limit N] <playlist_url> | --video <url>", file=sys.stderr)
        return 2
    from youtube_transcript_api import IpBlocked, RequestBlocked
    results = []
    videos = enumerate_playlist(args[0])
    if limit:
        videos = videos[:limit]
    for video_id, title in videos:
        try:
            segments, _language = transcript(video_id)
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
