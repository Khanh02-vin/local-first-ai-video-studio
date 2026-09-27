<script lang="ts">
  import { isTauri, invoke } from "@tauri-apps/api/core";
  type Playlist = { id: string; playlistId: string; playlistUrl: string; title: string | null; status: string; createdAt: string; updatedAt: string };
  type Result = { score: number; video_id: string; video_url: string; text: string; start_seconds: number; end_seconds: number; timestampUrl: string; playlist_id?: string; topic?: string };
  let playlistUrl = $state("");
  let playlistTitle = $state("");
  let playlists = $state<Playlist[]>([]);
  let question = $state("");
  let videoId = $state("");
  let playlistId = $state("");
  let topic = $state("");
  let topK = $state(5);
  let answer = $state("");
  let results = $state<Result[]>([]);
  let status = $state("Ready to search indexed knowledge.");
  let statusKind = $state<"idle" | "crawl" | "search" | "ingest">("idle");
  let loading = $state(false);
  let adding = $state(false);
  let showIngest = $state(false);
  let showFilters = $state(false);
  let ingestUrl = $state("");
  let ingestTitle = $state("");
  let ingestText = $state("");
  // Kênh của tôi (Option C — OAuth own-channel)
  let ytConnected = $state(false);
  let ytVideos = $state<{ videoId: string; title: string; publishedAt: string }[]>([]);
  let ytBusy = $state(false);
  let ytIngestId = $state("");
  let ytStatus = $state("");

  async function refreshYtConnection() {
    if (!isTauri()) return;
    try { const s = await invoke<{ connected: boolean }>("youtube_oauth_status"); ytConnected = s.connected; if (s.connected && !ytVideos.length) await loadOwnVideos(); } catch { /* browser */ }
  }
  async function connectYouTube() {
    if (!isTauri()) { ytStatus = "OAuth chạy trong app desktop — tab browser không kết nối được."; return; }
    ytBusy = true; ytStatus = "Mở trình duyệt để đăng nhập Google…";
    try {
      await invoke("youtube_oauth_start");
      for (let i = 0; i < 120; i++) {
        await new Promise(r => setTimeout(r, 1500));
        const s = await invoke<{ connected: boolean }>("youtube_oauth_status");
        if (s.connected) { ytConnected = true; ytStatus = "Đã kết nối kênh YouTube."; await loadOwnVideos(); return; }
      }
      ytStatus = "Hết thời gian chờ đăng nhập — thử lại.";
    } catch (e) { ytStatus = String(e); } finally { ytBusy = false; }
  }
  async function loadOwnVideos() {
    try { const data = await invoke<{ channel: string; videos: { videoId: string; title: string; publishedAt: string }[] }>("youtube_channel_videos"); ytVideos = data.videos; ytStatus = `Kênh: ${data.channel} · ${data.videos.length} video`; }
    catch (e) { ytStatus = String(e); }
  }
  async function ingestOwn(v: { videoId: string; title: string }) {
    ytIngestId = v.videoId;
    try { const r = await invoke<{ videoId: string; segments: number }>("youtube_ingest_captions", { videoId: v.videoId }); ytStatus = `Indexed ${r.segments} segments từ "${v.title}" — tìm được bằng ô search.`; }
    catch (e) { ytStatus = String(e); } finally { ytIngestId = ""; }
  }
  async function disconnectYouTube() { await invoke("youtube_oauth_disconnect").catch(() => {}); ytConnected = false; ytVideos = []; ytStatus = "Đã ngắt kết nối kênh."; }

  // In the desktop app the static webview (origin tauri://localhost) cannot
  // fetch the RAG sidecar directly — webview CSP is default-src 'self'. So we
  // resolve the sidecar base URL from Tauri and route through tauri-plugin-http,
  // whose HTTP runs in Rust and bypasses webview CSP/CORS. Standalone (plain
  // browser served by the adapter-node build) keeps same-origin relative fetch.
  let ragBase = "";
  let tauriFetch: typeof globalThis.fetch | null = null;

  async function initRag(): Promise<void> {
    if (!isTauri()) return;
    try {
      const { fetch: tFetch } = await import("@tauri-apps/plugin-http");
      tauriFetch = tFetch as typeof globalThis.fetch;
      ragBase = await invoke<string>("rag_server_url"); // spawns the sidecar if needed
    } catch (error) {
      status = String(error);
      statusKind = "crawl";
    }
  }

  async function rag(path: string, init?: RequestInit): Promise<Response> {
    const doFetch = tauriFetch ?? globalThis.fetch;
    return doFetch(ragBase + path, init);
  }

  async function loadPlaylists() {
    try { const response = await rag("/api/rag/playlists"); if (!response.ok) throw new Error("Playlist service unavailable"); playlists = (await response.json()).playlists ?? []; } catch (error) { status = String(error); statusKind = "crawl"; }
  }
  async function addPlaylist() {
    if (!playlistUrl.trim()) return;
    if (!isTauri()) { status = "Crawling runs in the desktop app only — this browser preview cannot reach YouTube. Use Manual transcript or open the installed app."; statusKind = "crawl"; return; }
    adding = true;
    statusKind = "crawl";
    try {
      // Queue the record in the sidecar, then run the local crawler and ingest results.
      const queued: { id: string; status: string } =
        await (await rag("/api/rag/playlists", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ playlistUrl, title: playlistTitle || undefined }) })).json();
      if (!queued.id) throw new Error("Playlist not queued");
      status = "Crawling playlist…";
      let result: { videos: number; indexed: number };
      try {
        result = await invoke("crawl_playlist", { playlistUrl, limit: 0 });
        status = `Indexed ${result.indexed}/${result.videos} videos from this playlist.`;
        await rag("/api/rag/playlists/status", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ id: queued.id, status: result.indexed ? "indexed" : "failed" }) });
      } catch (error) {
        status = `Crawl failed: ${String(error)}`;
        await rag("/api/rag/playlists/status", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ id: queued.id, status: "failed" }) }).catch(() => {});
      }
      playlistUrl = ""; playlistTitle = "";
      await loadPlaylists();
    } catch (error) { status = String(error); } finally { adding = false; }
  }
  function clearFilters() { videoId = ""; playlistId = ""; topic = ""; }
  let confirmDeleteId = $state("");
  async function deletePlaylist(id: string) {
    // Two-step confirm: first click arms ("Xóa?"), second click within 3s deletes.
    if (confirmDeleteId !== id) {
      confirmDeleteId = id;
      setTimeout(() => { if (confirmDeleteId === id) confirmDeleteId = ""; }, 3000);
      return;
    }
    confirmDeleteId = "";
    try { const response = await rag(`/api/rag/playlists/${id}`, { method: "DELETE" }); if (!response.ok) throw new Error((await response.json()).message ?? "Delete failed"); if (playlistId === id) playlistId = ""; status = "Playlist removed."; statusKind = "crawl"; await loadPlaylists(); } catch (error) { status = String(error); statusKind = "crawl"; }
  }
  const filtersActive = $derived(Boolean(videoId.trim() || playlistId.trim() || topic.trim()));
  const filterCount = $derived([videoId.trim(), playlistId.trim(), topic.trim()].filter(Boolean).length);
  async function search() {
    if (!question.trim()) return;
    loading = true; statusKind = "search"; status = "Searching indexed transcripts…"; answer = ""; results = [];
    try { const response = await rag("/api/rag/query", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ question, topK, videoId: videoId || undefined, playlistId: playlistId || undefined, topic: topic || undefined, synthesize: true }) }); if (!response.ok) throw new Error((await response.json()).message ?? "Search failed"); const data = await response.json(); answer = data.answer ?? "No synthesis returned."; results = data.results ?? []; status = `${results.length} timestamped excerpts found.`; } catch (error) { status = String(error); } finally { loading = false; }
  }
  async function ingest() {
    if (!ingestUrl.trim() || !ingestText.trim()) return;
    statusKind = "ingest";
    try { const segments = ingestText.split(/\n+/).map((line) => { const match = line.match(/^\s*(\d+(?:\.\d+)?)\s*-\s*(\d+(?:\.\d+)?)\s*\|\s*(.+)$/); return match ? { start: Number(match[1]), end: Number(match[2]), text: match[3] } : null; }).filter((item): item is { start: number; end: number; text: string } => item !== null); if (!segments.length) throw new Error("Use one segment per line: 12-18 | spoken text"); const response = await rag("/api/rag/ingest", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ videoUrl: ingestUrl, title: ingestTitle || undefined, segments }) }); if (!response.ok) throw new Error((await response.json()).message ?? "Ingest failed"); status = `Indexed ${segments.length} transcript segments.`; showIngest = false; } catch (error) { status = String(error); }
  }
  initRag().then(loadPlaylists);
  refreshYtConnection();
</script>

<svelte:head><title>YouTube Studio · Local-first AI Video Studio</title></svelte:head>

<main class="bench bench-duo youtube-workspace">
  <!-- Cột 1: nguồn dữ liệu -->
  <aside class="bench-left">
    <div class="panel"><span class="eyebrow">Knowledge source</span><h1>YouTube Studio</h1><p class="note">Index public playlists, then search their transcripts with timestamp citations.</p></div>
    <div class="panel"><div class="panel-head"><span class="panel-title"><span class="eyebrow">Kênh của tôi</span></span>{#if ytConnected}<span class="badge ok">connected</span>{/if}</div>
      {#if !ytConnected}
        <p class="note">Kết nối kênh YouTube của bạn (OAuth) để lấy transcript qua API chính chủ — hợp lệ, không scrape.</p>
        <button class="btn btn-primary btn-wide" disabled={ytBusy} onclick={connectYouTube}>{ytBusy ? "Đang chờ đăng nhập…" : "Kết nối kênh YouTube"}</button>
      {:else}
        <div class="stack">
          {#if !ytVideos.length}<p class="note">Kênh chưa có video nào.</p>{/if}
          {#each ytVideos as v}
            <div class="doc-item">
              <div class="spread"><strong>{v.title}</strong><button class="btn btn-sm btn-quiet" disabled={ytIngestId === v.videoId} onclick={() => ingestOwn(v)}>{ytIngestId === v.videoId ? "Đang lấy…" : "📄 Lấy transcript"}</button></div>
              <span class="note">{v.publishedAt.slice(0, 10)}</span>
            </div>
          {/each}
        </div>
        <button class="btn btn-quiet btn-wide" onclick={disconnectYouTube}>Ngắt kết nối</button>
      {/if}
      {#if ytStatus}<div class="status-inline" aria-live="polite">{ytStatus}</div>{/if}
    </div>
    <div class="panel"><div class="panel-title"><span class="eyebrow">Add playlist</span></div><label class="field"><span class="field-label">Playlist URL</span><input aria-label="YouTube playlist URL" placeholder="https://youtube.com/playlist?..." bind:value={playlistUrl} /></label><button class="btn btn-primary btn-wide" disabled={adding || !playlistUrl.trim()} onclick={addPlaylist}>{adding ? "Queueing…" : "Queue crawl"}</button>{#if statusKind === "crawl"}<div class="status-inline" aria-live="polite">{status}</div>{/if}</div>
    <div class="panel"><div class="panel-head"><span class="panel-title"><span class="eyebrow">Playlists</span></span><button class="btn btn-sm btn-quiet" onclick={loadPlaylists}>Refresh</button></div><div class="stack">{#if !playlists.length}<p class="note">No playlists registered yet.</p>{:else}{#each playlists as playlist}<div class="doc-item playlist-item" class:on={playlist.id === playlistId}><button class="playlist-select" onclick={() => playlistId = playlist.playlistId}><span class="spread"><strong>{playlist.title || playlist.playlistId}</strong><span class="badge" class:ok={playlist.status === "indexed"}>{playlist.status}</span></span><span class="doc-text">{playlist.playlistUrl}</span></button><button class="playlist-delete" class:confirm={confirmDeleteId === playlist.id} aria-label={confirmDeleteId === playlist.id ? `Confirm delete ${playlist.title || playlist.playlistId}` : `Delete ${playlist.title || playlist.playlistId}`} onclick={(event) => { event.stopPropagation(); deletePlaylist(playlist.id); }}>{confirmDeleteId === playlist.id ? "Xóa?" : "✕"}</button></div>{/each}{/if}</div></div>
  </aside>

  <!-- Cột 2: search + filters gọn + kết quả -->
  <section class="bench-center"><div class="stage">
    <div class="stage-head"><div class="stage-title"><span class="badge volt">indexed search</span><span class="stage-name">Ask your video library</span></div></div>
    <div class="stage-body">
      <div class="query-row">
        <textarea aria-label="Search question" placeholder="What did the speaker say about launch timing?" bind:value={question} onkeydown={(event) => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) void search(); }}></textarea>
        <button class="btn btn-primary" disabled={loading || !question.trim()} onclick={search}>{loading ? "Searching…" : "Search"}</button>
      </div>

      <!-- filters: toggle + chips thay vì panel phơi ra -->
      <div class="filters-row">
        <button class="btn btn-sm btn-quiet" onclick={() => showFilters = !showFilters}>⚙ Filters{#if filterCount}&nbsp;·&nbsp;{filterCount} active{/if}</button>
        {#if videoId.trim()}<span class="chip">Video: {videoId.trim()}<button aria-label="Clear video filter" onclick={() => videoId = ""}>✕</button></span>{/if}
        {#if playlistId.trim()}<span class="chip">Playlist: {playlistId.trim()}<button aria-label="Clear playlist filter" onclick={() => playlistId = ""}>✕</button></span>{/if}
        {#if topic.trim()}<span class="chip">Topic: {topic.trim()}<button aria-label="Clear topic filter" onclick={() => topic = ""}>✕</button></span>{/if}
      </div>
      {#if showFilters}
        <div class="drawer">
          <label class="field"><span class="field-label">Video ID</span><input placeholder="11-character ID" bind:value={videoId} /></label>
          <label class="field"><span class="field-label">Playlist ID</span><input placeholder="playlist filter" bind:value={playlistId} /></label>
          <label class="field"><span class="field-label">Topic</span><input placeholder="topic metadata" bind:value={topic} /></label>
          <label class="field"><span class="field-label">Top results</span><input type="number" min="1" max="20" bind:value={topK} /></label>
          {#if filtersActive}<button class="btn btn-sm btn-quiet" style="align-self:end;" onclick={clearFilters}>Clear all</button>{/if}
        </div>
      {/if}

      {#if statusKind === "search"}<div class="message" aria-live="polite">{status}</div>{/if}
      {#if answer}<article class="card answer"><span class="eyebrow">Synthesis</span><p>{answer}</p></article>{/if}

      <div class="doc">
        <div class="panel-head"><span class="panel-title">Timestamped excerpts</span><span class="badge">{results.length}</span></div>
        {#if !results.length}<div class="empty-canvas"><span class="eyebrow">No excerpts yet</span><strong>Search across indexed transcripts</strong><span class="note">Results keep the source video and exact jump time attached.</span></div>{:else}{#each results as result}<article class="doc-item"><div class="spread"><span class="badge volt">{Math.round(result.score * 100)}% match</span><a class="btn btn-sm btn-quiet" href={result.timestampUrl} target="_blank" rel="noreferrer">Open at {Math.floor(result.start_seconds)}s ↗</a></div><p class="doc-text">{result.text}</p><span class="note">{result.video_id} · {result.start_seconds.toFixed(1)}–{result.end_seconds.toFixed(1)}s</span></article>{/each}{/if}
      </div>
    </div>
    <div class="stage-foot">
      <button class="stage-foot-link" onclick={() => showIngest = true}>+ Paste a transcript manually</button>
      <span title="This workspace indexes public YouTube knowledge only — no channel, analytics, uploads, or private videos.">ⓘ</span>
    </div>
  </div></section>

  <!-- manual ingest: modal render có điều kiện (tránh hydration claim lệch với modal luôn-tồn-tại ở root) -->
  {#if showIngest}
    <div class="modal-back open" role="dialog" aria-modal="true" onclick={(event) => { if (event.target === event.currentTarget) showIngest = false; }}>
      <div class="modal">
        <div class="panel-head"><strong>Paste a transcript manually</strong><button class="btn btn-sm btn-quiet" aria-label="Close manual transcript" onclick={() => showIngest = false}>✕</button></div>
        <p class="note" style="margin:0;">Bring your own ordered segments when a source is not in a crawlable playlist.</p>
        <label class="field"><span class="field-label">Video URL</span><input placeholder="https://youtube.com/watch?v=…" bind:value={ingestUrl} /></label>
        <label class="field"><span class="field-label">Title</span><input placeholder="Video title" bind:value={ingestTitle} /></label>
        <label class="field"><span class="field-label">Segments</span><textarea rows="4" placeholder="12-18 | spoken text" bind:value={ingestText}></textarea></label>
        <button class="btn btn-primary btn-wide" onclick={() => ingest()}>Index segments</button>
        {#if statusKind === "ingest"}<div class="status-inline" aria-live="polite">{status}</div>{/if}
      </div>
    </div>
  {/if}
</main>
