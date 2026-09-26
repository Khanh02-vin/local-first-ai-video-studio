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
  let loading = $state(false);
  let adding = $state(false);
  let showIngest = $state(false);
  let ingestUrl = $state("");
  let ingestTitle = $state("");
  let ingestText = $state("");

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
    }
  }

  async function rag(path: string, init?: RequestInit): Promise<Response> {
    const doFetch = tauriFetch ?? globalThis.fetch;
    return doFetch(ragBase + path, init);
  }

  async function loadPlaylists() {
    try { const response = await rag("/api/rag/playlists"); if (!response.ok) throw new Error("Playlist service unavailable"); playlists = (await response.json()).playlists ?? []; } catch (error) { status = String(error); }
  }
  async function addPlaylist() {
    if (!playlistUrl.trim()) return;
    if (!isTauri()) { status = "Crawling runs in the desktop app only — this browser preview cannot reach YouTube. Use Manual transcript or open the installed app."; return; }
    adding = true;
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
  const filtersActive = $derived(Boolean(videoId.trim() || playlistId.trim() || topic.trim()));
  async function search() {
    if (!question.trim()) return;
    loading = true; status = "Searching indexed transcripts…"; answer = ""; results = [];
    try { const response = await rag("/api/rag/query", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ question, topK, videoId: videoId || undefined, playlistId: playlistId || undefined, topic: topic || undefined, synthesize: true }) }); if (!response.ok) throw new Error((await response.json()).message ?? "Search failed"); const data = await response.json(); answer = data.answer ?? "No synthesis returned."; results = data.results ?? []; status = `${results.length} timestamped excerpts found.`; } catch (error) { status = String(error); } finally { loading = false; }
  }
  async function ingest() {
    if (!ingestUrl.trim() || !ingestText.trim()) return;
    try { const segments = ingestText.split(/\n+/).map((line) => { const match = line.match(/^\s*(\d+(?:\.\d+)?)\s*-\s*(\d+(?:\.\d+)?)\s*\|\s*(.+)$/); return match ? { start: Number(match[1]), end: Number(match[2]), text: match[3] } : null; }).filter((item): item is { start: number; end: number; text: string } => item !== null); if (!segments.length) throw new Error("Use one segment per line: 12-18 | spoken text"); const response = await rag("/api/rag/ingest", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ videoUrl: ingestUrl, title: ingestTitle || undefined, segments }) }); if (!response.ok) throw new Error((await response.json()).message ?? "Ingest failed"); status = `Indexed ${segments.length} transcript segments.`; showIngest = false; } catch (error) { status = String(error); }
  }
  initRag().then(loadPlaylists);
</script>

<svelte:head><title>YouTube Studio · Local-first AI Video Studio</title></svelte:head>

<main class="bench youtube-workspace">
  <aside class="bench-left">
    <div class="panel"><div class="panel-head"><span class="eyebrow">Knowledge source</span><span class="badge ok">public URLs</span></div><h1>YouTube Studio</h1><p class="note">Index public playlists, then search their transcripts with timestamp citations.</p></div>
    <div class="panel"><div class="panel-head"><span class="panel-title">Add playlist</span></div><input aria-label="YouTube playlist URL" placeholder="https://youtube.com/playlist?..." bind:value={playlistUrl} /><input aria-label="Playlist title" placeholder="Optional label" bind:value={playlistTitle} /><button class="btn btn-primary btn-wide" disabled={adding || !playlistUrl.trim()} onclick={addPlaylist}>{adding ? "Queueing…" : "Queue crawl"}</button></div>
    <div class="panel"><div class="panel-head"><span class="panel-title">Playlists</span><button class="btn btn-sm btn-quiet" onclick={loadPlaylists}>Refresh</button></div><div class="stack">{#if !playlists.length}<p class="note">No playlists registered yet.</p>{:else}{#each playlists as playlist}<button class="doc-item" class:on={playlist.id === playlistId} onclick={() => playlistId = playlist.playlistId}><span class="spread"><strong>{playlist.title || playlist.playlistId}</strong><span class="badge" class:ok={playlist.status === "indexed"}>{playlist.status}</span></span><span class="doc-text">{playlist.playlistUrl}</span></button>{/each}{/if}</div></div>
  </aside>

  <section class="bench-center"><div class="stage"><div class="stage-head"><div class="stage-title"><span class="badge volt">indexed search</span><span class="stage-name">Ask your video library</span></div><span class="badge">local workspace</span></div><div class="stage-body"><div class="query-box"><textarea aria-label="Search question" placeholder="What did the speaker say about launch timing?" bind:value={question} onkeydown={(event) => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) void search(); }}></textarea><div class="spread"><span class="note">Ctrl / ⌘ + Enter to search</span><button class="btn btn-primary" disabled={loading || !question.trim()} onclick={search}>{loading ? "Searching…" : "Search library"}</button></div></div>{#if answer}<article class="card answer"><span class="eyebrow">Synthesis</span><p>{answer}</p></article>{/if}<div class="doc"><div class="panel-head"><span class="panel-title">Timestamped excerpts</span><span class="badge">{results.length}</span></div>{#if !results.length}<div class="empty-canvas"><span class="eyebrow">No excerpts yet</span><strong>Search across indexed transcripts</strong><span class="note">Results keep the source video and exact jump time attached.</span></div>{:else}{#each results as result}<article class="doc-item"><div class="spread"><span class="badge volt">{Math.round(result.score * 100)}% match</span><a class="btn btn-sm btn-quiet" href={result.timestampUrl} target="_blank" rel="noreferrer">Open at {Math.floor(result.start_seconds)}s ↗</a></div><p class="doc-text">{result.text}</p><span class="note">{result.video_id} · {result.start_seconds.toFixed(1)}–{result.end_seconds.toFixed(1)}s</span></article>{/each}{/if}</div></div></div></section>

  <aside class="bench-right"><div class="panel"><div class="panel-head"><span class="panel-title">Filters</span>{#if filtersActive}<span class="badge volt">filters active</span>{:else}<span class="badge">optional</span>{/if}</div>{#if filtersActive}<div class="spread"><span class="note">Filters narrow the search below.</span><button class="btn btn-sm btn-quiet" onclick={clearFilters}>Clear</button></div>{/if}<label class="field"><span class="field-label">Video ID</span><input placeholder="11-character ID" bind:value={videoId} /></label><label class="field"><span class="field-label">Playlist ID</span><input placeholder="playlist filter" bind:value={playlistId} /></label><label class="field"><span class="field-label">Topic</span><input placeholder="topic metadata" bind:value={topic} /></label><label class="field"><span class="field-label">Top results</span><input type="number" min="1" max="20" bind:value={topK} /></label></div><div class="panel"><div class="panel-head"><span class="panel-title">Manual transcript</span><button class="btn btn-sm btn-quiet" onclick={() => showIngest = !showIngest}>{showIngest ? "Close" : "Open"}</button></div><p class="note">Bring your own ordered segments when a source is not in a crawlable playlist.</p>{#if showIngest}<div class="stack"><input placeholder="YouTube video URL" bind:value={ingestUrl} /><input placeholder="Video title" bind:value={ingestTitle} /><textarea placeholder="12-18 | spoken text" bind:value={ingestText}></textarea><button class="btn btn-wide" onclick={ingest}>Index segments</button></div>{/if}</div><div class="panel"><span class="eyebrow">Service note</span><p class="note">This workspace indexes public YouTube knowledge. It does not connect to a channel, analytics, uploads, or private videos.</p><div class="message" aria-live="polite">{status}</div></div></aside>
</main>
