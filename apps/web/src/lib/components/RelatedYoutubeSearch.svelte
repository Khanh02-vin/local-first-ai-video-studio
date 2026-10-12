<script lang="ts">
  import { invoke, isTauri } from "@tauri-apps/api/core";

  type Video = { videoId: string; title: string; thumbnail: string; url: string };

  let { initialQuery = "" }: { initialQuery?: string } = $props();
  let query = $state(initialQuery);
  let videos = $state<Video[]>([]);
  let busy = $state(false);
  let status = $state("");

  async function search() {
    if (!query.trim()) return;
    if (!isTauri()) { status = "Public YouTube search is available in the desktop app only."; return; }
    busy = true; status = "Searching public YouTube videos…"; videos = [];
    try {
      const result = await invoke<{ videos: Video[] }>("youtube_search_videos", { query });
      videos = result.videos;
      status = `${videos.length} public videos found.`;
    } catch (error) { status = String(error); }
    finally { busy = false; }
  }

  async function copy(url: string) {
    try { await navigator.clipboard.writeText(url); status = "Video URL copied."; }
    catch (error) { status = `Could not copy URL: ${String(error)}`; }
  }
</script>

<section class="card" aria-label="Related public YouTube videos">
  <div class="section-title">Related public YouTube videos</div>
  <p class="hint">Search public video titles and thumbnails. No videos are downloaded.</p>
  <form onsubmit={(event) => { event.preventDefault(); void search(); }} style="display:flex;gap:.5rem;">
    <input aria-label="Search related YouTube videos" placeholder="Search YouTube videos…" bind:value={query} style="flex:1;min-width:0;" />
    <button class="browse" type="submit" disabled={busy || !query.trim()}>{busy ? "Searching…" : "Search"}</button>
  </form>
  {#if status}<p class="hint" aria-live="polite">{status}</p>{/if}
  {#if videos.length}
    <div class="stack" style="margin-top:.75rem;">
      {#each videos as video (video.videoId)}
        <div class="preview-card">
          {#if video.thumbnail}<img src={video.thumbnail} alt="" class="youtube-thumb" />{/if}
          <div class="preview-info"><strong>{video.title}</strong><span>{video.videoId}</span></div>
          <a class="btn btn-sm btn-quiet" href={video.url} target="_blank" rel="noreferrer">Open ↗</a>
          <button class="ghost" type="button" onclick={() => copy(video.url)}>Copy URL</button>
        </div>
      {/each}
    </div>
  {/if}
</section>
