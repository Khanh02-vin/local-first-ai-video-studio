// The YouTube branch: search, OAuth for the user's own channel, caption ingest,
// playlist crawling, and the local proxy that serves a YouTube preview.
//
// Moved out of main.rs so the whole YouTube surface can be read (and changed) as one
// piece: the commands, their helpers and the preview proxy belong together.
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths::{
    analysis_range_allowed, helper_env, helper_script, jobs_path, node_bin, python_binary,
    runtime_env, data_dir, tool, venv_python, whisper_venv_bin, write_private_file,
};
use crate::runtime::{ensure_rag_server, RagState};
use crate::paths::NODE_ARGS;
use crate::{
    spawn_analysis_worker, Jobs,
};

/// Creates the app venv with only the YouTube crawler deps (yt-dlp +
/// youtube-transcript-api). Returns false when the venv is unusable — local
/// file analysis does not depend on it.
pub(crate) fn prepare_youtube_venv(venv: &Path) -> bool {
    if std::fs::create_dir_all(data_dir()).is_err() {
        return false;
    }
    let created = tool(&python_binary())
        .args(["-m", "venv"])
        .arg(venv)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !created {
        return false;
    }
    let pip = whisper_venv_bin("pip");
    let _ = tool(&pip).args(["install", "--upgrade", "pip"]).output();
    tool(&pip)
        .args(["install", "yt-dlp", "youtube-transcript-api"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

// --- Public YouTube search ------------------------------------------------------
pub(crate) fn youtube_data_api_key() -> Result<String, String> {
    std::env::var("YOUTUBE_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| {
            runtime_env()
                .get("YOUTUBE_API_KEY")
                .cloned()
                .filter(|key| !key.trim().is_empty())
        })
        .ok_or_else(|| {
            "YT_API_KEY_MISSING: set YOUTUBE_API_KEY in the app runtime environment or runtime.env"
                .into()
        })
}

pub(crate) fn youtube_search_query(query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("YT_SEARCH_QUERY_EMPTY".into());
    }
    if query.chars().count() > 200 {
        return Err("YT_SEARCH_QUERY_TOO_LONG: maximum 200 characters".into());
    }
    Ok(query.to_string())
}

#[tauri::command]
pub(crate) fn youtube_search_videos(query: String) -> Result<serde_json::Value, String> {
    let query = youtube_search_query(&query)?;
    let key = config_value(&youtube_data_api_key()?);
    // Keep the key out of the curl process arguments; curl reads its URL-encoded
    // parameters from stdin, and its config's quoted-value syntax is escaped here.
    fn config_value(value: &str) -> String {
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    }
    use std::io::Write;
    let mut child = tool("curl")
        .args([
            "-sS",
            "--get",
            "https://www.googleapis.com/youtube/v3/search",
            "--config",
            "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("YT_SEARCH_SPAWN:{e}"))?;
    let config = format!("data-urlencode = \"key={}\"\ndata-urlencode = \"part=snippet\"\ndata-urlencode = \"type=video\"\ndata-urlencode = \"maxResults=8\"\ndata-urlencode = \"q={}\"\n", config_value(&key), config_value(&query));
    child
        .stdin
        .take()
        .ok_or("YT_SEARCH_STDIN")?
        .write_all(config.as_bytes())
        .map_err(|e| format!("YT_SEARCH_WRITE:{e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("YT_SEARCH_WAIT:{e}"))?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        format!(
            "YT_SEARCH_RESPONSE:{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    if let Some(error) = value.get("error") {
        let reason = error["errors"][0]["reason"].as_str().unwrap_or("");
        return Err(
            if ["quotaExceeded", "dailyLimitExceeded", "rateLimitExceeded"].contains(&reason) {
                format!(
                    "YT_SEARCH_QUOTA: YouTube Data API quota exceeded ({})",
                    error["message"].as_str().unwrap_or(reason)
                )
            } else {
                format!(
                    "YT_SEARCH_API: {}",
                    error["message"].as_str().unwrap_or("request failed")
                )
            },
        );
    }
    let videos: Vec<serde_json::Value> = value["items"].as_array().into_iter().flatten().filter_map(|item| {
        let id = item["id"]["videoId"].as_str()?;
        Some(serde_json::json!({
            "videoId": id,
            "title": item["snippet"]["title"].as_str().unwrap_or("Untitled video"),
            "thumbnail": item["snippet"]["thumbnails"]["medium"]["url"].as_str().or(item["snippet"]["thumbnails"]["default"]["url"].as_str()).unwrap_or(""),
            "url": format!("https://www.youtube.com/watch?v={id}"),
        }))
    }).collect();
    Ok(serde_json::json!({ "videos": videos }))
}

// --- YouTube "own channel" mode (Option C) -------------------------------------
// Customer connects their own Google account once (OAuth loopback flow in the
// system browser); the app then lists their uploads and pulls captions through
// the official API — legitimate for owned content, no scraping.
pub(crate) const YT_OAUTH_PORT: u16 = 14871;

pub(crate) const YT_SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct YtTokens {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
}

pub(crate) fn yt_tokens_file() -> PathBuf {
    data_dir().join("youtube-tokens.json")
}

pub(crate) fn yt_client_creds() -> Result<(String, String), String> {
    // Developer ships credentials via build env; a state-dir file overrides for
    // local testing so customers never see either value.
    let file = data_dir().join("youtube-client.json");
    if let Ok(text) = std::fs::read_to_string(&file) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let id = v["clientId"].as_str().unwrap_or("").to_string();
            let secret = v["clientSecret"].as_str().unwrap_or("").to_string();
            if !id.is_empty() && !secret.is_empty() {
                return Ok((id, secret));
            }
        }
    }
    let id = option_env!("GOOGLE_CLIENT_ID").unwrap_or("").to_string();
    let secret = option_env!("GOOGLE_CLIENT_SECRET")
        .unwrap_or("")
        .to_string();
    if id.is_empty() || secret.is_empty() {
        return Err("YT_CLIENT_MISSING: create a Google OAuth Desktop client and save {\"clientId\",\"clientSecret\"} to youtube-client.json in the app state dir".into());
    }
    Ok((id, secret))
}

pub(crate) fn yt_load_tokens() -> Option<YtTokens> {
    serde_json::from_str(&std::fs::read_to_string(yt_tokens_file()).ok()?).ok()
}

pub(crate) fn yt_save_tokens(tokens: &YtTokens) {
    let _ = write_private_file(
        &yt_tokens_file(),
        serde_json::to_string(tokens).unwrap_or_default().as_bytes(),
    );
}

pub(crate) fn b64url_nopad(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(T[(n >> 12) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

pub(crate) fn yt_token_request(form: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let mut cmd = tool("curl");
    cmd.args(["-s", "-X", "POST", "https://oauth2.googleapis.com/token"]);
    for (k, v) in form {
        cmd.arg("--data-urlencode").arg(format!("{k}={v}"));
    }
    let out = cmd.output().map_err(|e| format!("CURL_SPAWN:{e}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("TOKEN_JSON:{e}"))?;
    if value.get("error").is_some() {
        return Err(format!(
            "TOKEN_ERROR:{}",
            value["error"].as_str().unwrap_or("?")
        ));
    }
    Ok(value)
}

pub(crate) fn yt_refresh_if_needed(tokens: &mut YtTokens) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if tokens.expires_at > now + 120 {
        return Ok(());
    }
    let (client_id, client_secret) = yt_client_creds()?;
    let value = yt_token_request(&[
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", tokens.refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ])?;
    tokens.access_token = value["access_token"].as_str().unwrap_or("").to_string();
    let expires = value["expires_in"].as_u64().unwrap_or(3600);
    tokens.expires_at = now + expires.saturating_sub(60);
    yt_save_tokens(tokens);
    Ok(())
}

pub(crate) fn yt_bearer() -> Result<String, String> {
    let mut tokens =
        yt_load_tokens().ok_or("YT_NOT_CONNECTED: connect your YouTube channel first")?;
    yt_refresh_if_needed(&mut tokens)?;
    Ok(tokens.access_token)
}

pub(crate) fn yt_api_get(path_with_query: &str) -> Result<serde_json::Value, String> {
    let bearer = yt_bearer()?;
    let out = tool("curl")
        .args(["-s", "-H", &format!("Authorization: Bearer {bearer}")])
        .arg(format!("https://www.googleapis.com{path_with_query}"))
        .output()
        .map_err(|e| format!("YT_API_SPAWN:{e}"))?;
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| {
        format!(
            "YT_API_JSON:{e} — {}",
            String::from_utf8_lossy(&out.stdout)
                .chars()
                .take(120)
                .collect::<String>()
        )
    })?;
    if let Some(err) = value.get("error") {
        return Err(format!("YT_API:{}", err["message"].as_str().unwrap_or("?")));
    }
    Ok(value)
}

#[tauri::command]
pub(crate) fn youtube_oauth_status() -> serde_json::Value {
    match yt_load_tokens() {
        Some(mut tokens) => {
            let connected = yt_refresh_if_needed(&mut tokens).is_ok();
            serde_json::json!({ "connected": connected, "expiresAt": tokens.expires_at })
        }
        None => serde_json::json!({ "connected": false }),
    }
}

#[tauri::command]
pub(crate) fn youtube_oauth_disconnect() -> Result<(), String> {
    std::fs::remove_file(yt_tokens_file()).unwrap_or(());
    Ok(())
}

#[tauri::command]
pub(crate) fn youtube_oauth_start() -> Result<String, String> {
    let client_id = yt_client_creds()?.0;
    // Deterministic PKCE verifier; challenge hashed with the same sha256sum CLI
    // the rest of this file already shells out to.
    let verifier = format!(
        "{}{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let digest = tool("sha256sum")
        .arg(&verifier)
        .output()
        .map_err(|e| format!("SHA_SPAWN:{e}"))?;
    let raw = String::from_utf8_lossy(&digest.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    let challenge = b64url_nopad(&raw.as_bytes());
    let redirect = format!("http://127.0.0.1:{YT_OAUTH_PORT}/callback");
    let scope_enc = YT_SCOPE.replace(':', "%3A").replace('/', "%2F");
    let consent_url = format!("https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}&redirect_uri={redirect}&response_type=code&scope={scope_enc}&access_type=offline&prompt=consent&include_granted_scopes=true&code_challenge={challenge}&code_challenge_method=S256");
    // The loopback listener must exist BEFORE the browser navigates.
    let listener = std::net::TcpListener::bind(("127.0.0.1", YT_OAUTH_PORT))
        .map_err(|e| format!("YT_PORT_BUSY:{e} — an OAuth callback is already waiting"))?;
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = String::new();
            use std::io::{Read, Write};
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            let _ = stream.read_to_string(&mut request);
            let code = request
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.split("code=").nth(1))
                .map(|rest| rest.split('&').next().unwrap_or("").to_string())
                .unwrap_or_default();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body style='font-family:sans-serif;background:#11100e;color:#f4efe4'><h2>&#9989; YouTube connected</h2><p>B&#7841;n c&oacute; th&#7875; &#273;&oacute;ng tab n&agrave;y v&agrave; quay l&#7841;i app.</p></body></html>");
            let _ = stream.flush();
            drop(stream);
            if code.is_empty() {
                return;
            }
            if let Ok((client_id, client_secret)) = yt_client_creds() {
                if let Ok(value) = yt_token_request(&[
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("code", code.as_str()),
                    ("grant_type", "authorization_code"),
                    ("redirect_uri", redirect.as_str()),
                    ("code_verifier", verifier.as_str()),
                ]) {
                    let access = value["access_token"].as_str().unwrap_or("").to_string();
                    let refresh = value["refresh_token"].as_str().unwrap_or("").to_string();
                    let expires = value["expires_in"].as_u64().unwrap_or(3600);
                    if !access.is_empty() && !refresh.is_empty() {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        yt_save_tokens(&YtTokens {
                            access_token: access,
                            refresh_token: refresh,
                            expires_at: now + expires.saturating_sub(60),
                        });
                    }
                }
            }
        }
    });
    // Open the system browser for the consent screen.
    let opened = if cfg!(target_os = "linux") {
        tool("xdg-open").arg(&consent_url).spawn().is_ok()
    } else if cfg!(target_os = "macos") {
        tool("open").arg(&consent_url).spawn().is_ok()
    } else {
        tool("cmd")
            .args(["/C", "start"])
            .arg(&consent_url)
            .spawn()
            .is_ok()
    };
    if !opened {
        return Ok(format!("OPEN_MANUALLY:{consent_url}"));
    }
    Ok(consent_url)
}

#[tauri::command]
pub(crate) fn youtube_channel_videos() -> Result<serde_json::Value, String> {
    let channel = yt_api_get("/youtube/v3/channels?part=snippet,contentDetails&mine=true")?;
    let title = channel["items"][0]["snippet"]["title"]
        .as_str()
        .unwrap_or("Kênh của tôi")
        .to_string();
    let uploads = channel["items"][0]["contentDetails"]["relatedPlaylists"]["uploads"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if uploads.is_empty() {
        return Err("YT_NO_UPLOADS: kênh này chưa có video nào".into());
    }
    let items = yt_api_get(&format!(
        "/youtube/v3/playlistItems?part=snippet,contentDetails&maxResults=50&playlistId={uploads}"
    ))?;
    let videos: Vec<serde_json::Value> = items["items"].as_array().unwrap_or(&vec![]).iter().map(|item| {
        serde_json::json!({
            "videoId": item["contentDetails"]["videoId"],
            "title": item["snippet"]["title"],
            "publishedAt": item["contentDetails"]["videoPublishedAt"].as_str().or(item["snippet"]["publishedAt"].as_str()).unwrap_or(""),
            "thumbnail": item["snippet"]["thumbnails"]["medium"]["url"].as_str().unwrap_or(""),
        })
    }).collect();
    Ok(serde_json::json!({ "channel": title, "videos": videos }))
}

/// Parses SRT ("00:00:01,000 --> 00:00:04,000") into {start,end,text} segments.
pub(crate) fn parse_srt_segments(text: &str) -> Vec<serde_json::Value> {
    fn secs(stamp: &str) -> f64 {
        let parts: Vec<f64> = stamp
            .trim()
            .split(':')
            .filter_map(|p| p.replace(",", ".").parse::<f64>().ok())
            .collect();
        match parts.as_slice() {
            [h, m, s] => h * 3600.0 + m * 60.0 + s,
            [m, s] => m * 60.0 + s,
            [s] => *s,
            _ => 0.0,
        }
    }
    let mut segments = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < 2 {
            continue;
        }
        if let Some(arrow) = lines.iter().find(|l| l.contains("-->")) {
            let mut halves = arrow.split("-->");
            let start = secs(halves.next().unwrap_or(""));
            let end = secs(halves.next().unwrap_or("0"));
            let body: String = lines[lines.iter().position(|l| l.contains("-->")).unwrap() + 1..]
                .join(" ")
                .trim()
                .to_string();
            if !body.is_empty() && end > start {
                segments.push(serde_json::json!({ "start": (start * 100.0).round() / 100.0, "end": (end * 100.0).round() / 100.0, "text": body }));
            }
        }
    }
    segments
}

/// Downloads captions for an OWN video through the official API and ingests the
/// transcript into the local RAG store via the sidecar.
#[tauri::command]
pub(crate) fn youtube_ingest_captions(
    app: tauri::AppHandle,
    video_id: String,
) -> Result<serde_json::Value, String> {
    let bearer = yt_bearer()?;
    let caps = {
        let out = tool("curl")
            .args(["-s", "-H", &format!("Authorization: Bearer {bearer}")])
            .arg(format!(
                "https://www.googleapis.com/youtube/v3/captions?part=snippet&videoId={video_id}"
            ))
            .output()
            .map_err(|e| format!("YT_API_SPAWN:{e}"))?;
        let value: serde_json::Value =
            serde_json::from_slice(&out.stdout).map_err(|e| format!("YT_API_JSON:{e}"))?;
        if let Some(err) = value.get("error") {
            return Err(format!(
                "YT_CAPTIONS:{}",
                err["message"].as_str().unwrap_or("?")
            ));
        }
        value["items"].as_array().cloned().unwrap_or_default()
    };
    if caps.is_empty() {
        return Err("YT_NO_CAPTIONS: video này chưa có phụ đề nào trên YouTube (hãy bật phụ đề tự động hoặc tải lên phụ đề trước)".into());
    }
    let cap_id = caps[0]["id"].as_str().unwrap_or("").to_string();
    let srt = {
        let out = tool("curl")
            .args(["-s", "-L", "-H", &format!("Authorization: Bearer {bearer}")])
            .arg(format!(
                "https://www.googleapis.com/youtube/v3/captions/{cap_id}?tfmt=srt"
            ))
            .output()
            .map_err(|e| format!("CAP_SPAWN:{e}"))?;
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let segments = parse_srt_segments(&srt);
    if segments.is_empty() {
        return Err("CAP_PARSE_EMPTY: phụ đề tải về rỗng hoặc định dạng lạ".into());
    }
    let endpoint = ensure_rag_server(&RagState::default(), &app)?;
    let body = serde_json::json!({ "videoUrl": format!("https://www.youtube.com/watch?v={video_id}"), "segments": segments });
    let body_str = body.to_string();
    let out = tool("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "-H",
            &format!("Authorization: Bearer {}", endpoint.token),
            "-d",
            &body_str,
            &format!("{}/api/rag/ingest", endpoint.base),
        ])
        .output()
        .map_err(|e| format!("INGEST_SPAWN:{e}"))?;
    if !String::from_utf8_lossy(&out.stdout).trim().starts_with('2') {
        return Err(format!(
            "INGEST_HTTP:{}",
            String::from_utf8_lossy(&out.stdout).trim()
        ));
    }
    Ok(serde_json::json!({ "videoId": video_id, "segments": segments.len() }))
}

/// Extracts a video id from a youtube.com/youtu.be link for validation only —
/// the original (case-preserved) URL is what reaches yt-dlp afterwards.
pub(crate) fn youtube_video_id(value: &str) -> Option<String> {
    let url = url::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.host_str(), Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be"))
        || url.port().is_some()
    {
        return None;
    }
    let id = if url.host_str() == Some("youtu.be") {
        url.path_segments()?.next()?.to_string()
    } else if url.path() == "/watch" {
        url.query_pairs().find_map(|(key, value)| (key == "v").then(|| value.into_owned()))?
    } else {
        let mut parts = url.path_segments()?;
        let kind = parts.next()?;
        if !matches!(kind, "embed" | "shorts") { return None; }
        parts.next()?.to_string()
    };
    (id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .then_some(id)
}

/// Fetches (title, duration) for a YouTube URL — one metadata-only yt-dlp call (~1s).
pub(crate) fn yt_video_meta(app: &tauri::AppHandle, url: &str) -> Result<(String, f64), String> {
    let python = venv_python();
    if !python.is_file() {
        return Err("PYTHON_VENV_MISSING: run whisper bootstrap first".into());
    }
    let mut cmd = tool(&python);
    cmd.args([
        "-m",
        "yt_dlp",
        "--skip-download",
        "--no-playlist",
        "--print",
        "%(title)s\t%(duration)s",
    ])
    .arg(url);
    for (key, value) in helper_env(app, &[]) {
        cmd.env(key, value);
    }
    let output = cmd.output().map_err(|e| format!("YT_META_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "YT_META:{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or("");
    let (title, duration_text) = line.rsplit_once('\t').ok_or("YT_META_PARSE")?;
    let duration: f64 = duration_text
        .trim()
        .parse()
        .map_err(|_| "YT_NO_DURATION".to_string())?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err("YT_NO_DURATION".into());
    }
    Ok((title.to_string(), duration))
}

/// Resolves fresh CDN stream URLs at export time: YouTube URLs expire (~6h), so
/// they must never be cached between analysis and render. YouTube is DASH-only
/// nowadays, so this usually returns [videoUrl, audioUrl] for ffmpeg's two inputs.
pub(crate) fn resolve_stream_url(app: &tauri::AppHandle, url: &str) -> Result<Vec<String>, String> {
    let python = venv_python();
    if !python.is_file() {
        return Err("PYTHON_VENV_MISSING: run whisper bootstrap first".into());
    }
    let mut cmd = tool(&python);
    cmd.args([
        "-m",
        "yt_dlp",
        "--no-playlist",
        "-g",
        "-f",
        "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b[ext=mp4]/b",
    ])
    .arg(url);
    for (key, value) in helper_env(app, &[]) {
        cmd.env(key, value);
    }
    let output = cmd.output().map_err(|e| format!("YT_STREAM_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "YT_STREAM_RESOLVE:{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let urls: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if urls.is_empty() {
        return Err("YT_STREAM_EMPTY: yt-dlp returned no stream URL for this video".into());
    }
    Ok(urls)
}

/// Starts a text-only analysis job from a YouTube URL: metadata (~1s) → init →
/// analyze-youtube.ts worker (captions, no Whisper, no download).
#[tauri::command]
pub(crate) fn start_youtube_analysis(
    app: tauri::AppHandle,
    jobs: tauri::State<'_, Jobs>,
    url: String,
) -> Result<serde_json::Value, String> {
    let url = url.trim().to_string();
    if youtube_video_id(&url).is_none() {
        return Err("INVALID_YOUTUBE_URL".into());
    }
    let (title, duration) = yt_video_meta(&app, &url)?;
    if !analysis_range_allowed(0.0, duration) {
        return Err("ANALYZE_RANGE_EXCEEDS_600_SECONDS: select a shorter range".into());
    }
    let id = format!(
        "analysis-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "CLOCK_ERROR")?
            .as_nanos()
    );
    let init_script = helper_script(&app, "analysis-init.ts")?;
    let mut init = tool(node_bin(&app));
    init.args(NODE_ARGS)
        .arg(init_script)
        .arg(jobs_path())
        .arg(&id)
        .arg(&url)
        .arg(format!("{duration:.3}"))
        .arg("0")
        .arg(format!("{duration:.3}"));
    for (key, value) in helper_env(&app, &[]) {
        init.env(key, value);
    }
    let init = init.output().map_err(|e| format!("STORE_INIT:{e}"))?;
    if !init.status.success() {
        return Err(String::from_utf8_lossy(&init.stderr).trim().to_owned());
    }
    if !spawn_analysis_worker(
        app.clone(), jobs.inner().clone(), id.clone(), PathBuf::from(&url), duration, 0.0, duration,
    ) {
        let _ = crate::analysis_store::AnalysisStore::open(&jobs_path())
            .and_then(|store| store.fail(&id, "ANALYSIS_BUSY"));
        return Err("ANALYSIS_BUSY: another analysis is already running".into());
    }
    Ok(serde_json::json!({ "id": id, "duration": duration, "title": title }))
}

// --- YouTube preview shim -------------------------------------------------------
// The packaged webview origin is tauri://localhost; WebKitGTK sends no Referer
// for that scheme and YouTube rejects such embeds with error 153
// (EMBEDDER_IDENTITY_MISSING_REFERRER). A loopback page around the player
// gives it a normal http://localhost Referer, which YouTube accepts.
pub(crate) const YT_PREVIEW_PORT: u16 = 14872;

/// Per-session token every /preview request must carry. The preview shim proxies a
/// YouTube embed on loopback; without the token, a web page in the user's browser
/// could make the shim do that too. The port stays fixed because the webview CSP
/// allow-lists it statically (tauri.conf.json frame-src).
static PREVIEW_TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn preview_session_token() -> String {
    PREVIEW_TOKEN.get_or_init(crate::paths::session_token).clone()
}

/// Parses the shim's query into (videoId, start, end); anything else is rejected
/// so the served page is only ever built from validated pieces.
pub(crate) fn preview_query(query: &str) -> Option<(String, u32, u32)> {
    let mut id = String::new();
    let mut start: Option<u32> = None;
    let mut end: Option<u32> = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "id" => id = value.to_string(),
            "start" => start = value.parse().ok(),
            "end" => end = value.parse().ok(),
            _ => {}
        }
    }
    if id.len() != 11
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let (start, end) = (start?, end?);
    if start >= end {
        return None;
    }
    Some((id, start, end))
}

pub(crate) fn preview_page(video_id: &str, start: u32, end: u32) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0;height:100%;background:#000}}iframe{{display:block;width:100%;height:100%;border:0}}</style></head><body>\
         <iframe src=\"https://www.youtube.com/embed/{video_id}?start={start}&end={end}&rel=0\" allow=\"accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture\" allowfullscreen></iframe>\
         </body></html>"
    )
}

pub(crate) fn preview_serve(listener: std::net::TcpListener, token: String) {
    use std::io::{Read, Write};
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let token = token.clone();
        thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let mut request: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        request.extend_from_slice(&chunk[..n]);
                        if request.len() > 8192 || request.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let text = String::from_utf8_lossy(&request);
            let target = text.split_whitespace().nth(1).unwrap_or("");
            let (path, query) = target.split_once('?').unwrap_or((target, ""));
            // Every /preview request must carry this session's token; /health stays
            // open because it answers a constant string and nothing else.
            let authorized = query.split('&').any(|pair| pair == format!("token={token}").as_str());
            let (status, body) = match path {
                "/health" => ("200 OK", "yt-preview".to_string()),
                "/preview" if authorized => match preview_query(query) {
                    Some((id, start, end)) => ("200 OK", preview_page(&id, start, end)),
                    None => ("400 Bad Request", "bad preview request".to_string()),
                },
                "/preview" => ("403 Forbidden", "missing or wrong preview token".to_string()),
                _ => ("404 Not Found", "not found".to_string()),
            };
            let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            let _ = stream.flush();
        });
    }
}

/// Returns the loopback base URL for the YouTube preview iframe, starting the
/// shim on first use (reuses an instance already serving after a reload).
/// Returns the loopback /preview URL prefix, session token included, starting the
/// shim on first use (reuses an instance already serving after a reload). The UI
/// appends `&id=…&start=…&end=…` to this prefix.
#[tauri::command]
pub(crate) fn preview_origin() -> Result<String, String> {
    let token = preview_session_token();
    let prefix = format!("http://localhost:{YT_PREVIEW_PORT}/preview?token={token}");
    if let Ok(mut probe) = std::net::TcpStream::connect(("localhost", YT_PREVIEW_PORT)) {
        use std::io::{Read, Write};
        let _ = probe.set_read_timeout(Some(std::time::Duration::from_secs(2)));
        let _ = probe
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        let mut reply = String::new();
        let _ = probe.read_to_string(&mut reply);
        if reply.starts_with("HTTP/1.1 200") && reply.contains("yt-preview") {
            return Ok(prefix);
        }
        return Err(format!("PREVIEW_PORT_BUSY:{YT_PREVIEW_PORT}"));
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", YT_PREVIEW_PORT))
        .map_err(|e| format!("PREVIEW_PORT_BUSY:{e}"))?;
    thread::spawn(move || preview_serve(listener, token));
    Ok(prefix)
}

pub(crate) fn valid_playlist_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value.trim()) else { return false; };
    matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(url.host_str(), Some("youtube.com" | "www.youtube.com" | "m.youtube.com"))
        && url.path() == "/playlist"
        && {
            let ids: Vec<_> = url.query_pairs().filter_map(|(key, value)| (key == "list").then(|| value.into_owned())).collect();
            ids.len() == 1 && ids[0].len() >= 10 && ids[0].chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }
}

/// whisper venv, then stores the result via the RAG sidecar ingest endpoint.
/// Best-effort: a missing crawler/venv yields an explanatory error instead of a panic.
pub(crate) fn run_yt_crawler(
    app: &tauri::AppHandle,
    playlist_url: &str,
    limit: u32,
) -> Result<serde_json::Value, String> {
    if !valid_playlist_url(&playlist_url) {
        return Err("INVALID_YOUTUBE_PLAYLIST_URL".into());
    }
    let venv_python = venv_python();
    if !venv_python.is_file() {
        return Err("PYTHON_VENV_MISSING: run whisper bootstrap first".into());
    }

    let crawler = helper_script(app, "yt-crawler.py")?;
    let mut cmd = tool(&venv_python);
    cmd.arg(&crawler).arg(playlist_url);
    if limit > 0 {
        cmd.arg("--limit").arg(limit.to_string());
    }
    for (key, value) in helper_env(app, &[]) {
        cmd.env(key, value);
    }
    let output = cmd.output().map_err(|e| format!("CRAWLER_SPAWN:{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    // Crawler writes JSON to stdout: [{videoId,title,segments}]
    let videos: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("CRAWLER_JSON:{e}"))?;

    // Ingest each result via the running RAG sidecar (bundled curl — no new HTTP dep).
    let endpoint = ensure_rag_server(&RagState::default(), app)?;
    let ingest_url = format!("{}/api/rag/ingest", endpoint.base);
    let mut indexed = 0usize;
    for video in &videos {
        let body = serde_json::json!({
            "videoUrl": format!("https://www.youtube.com/watch?v={}", video["videoId"].as_str().unwrap_or_default()),
            "title": video.get("title").cloned(),
            "segments": video.get("segments").cloned().unwrap_or(serde_json::Value::Array(vec![])),
        });
        let body_str = body.to_string();
        let out = tool("curl")
            .args([
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "-X",
                "POST",
                "-H",
                "content-type: application/json",
                "-d",
                &body_str,
                &ingest_url,
            ])
            .output()
            .map_err(|e| format!("INGEST_SPAWN:{e}"))?;
        if !out.status.success() {
            continue;
        }
        if String::from_utf8_lossy(&out.stdout).trim().starts_with('2') {
            indexed += 1;
        }
    }
    Ok(serde_json::json!({ "videos": videos.len(), "indexed": indexed }))
}

#[tauri::command]
pub(crate) fn crawl_playlist(
    app: tauri::AppHandle,
    playlist_url: String,
    limit: u32,
) -> Result<serde_json::Value, String> {
    if !valid_playlist_url(&playlist_url) {
        return Err("INVALID_YOUTUBE_PLAYLIST_URL".into());
    }
    run_yt_crawler(&app, &playlist_url, limit)
}

