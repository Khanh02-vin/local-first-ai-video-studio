<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { page } from "$app/state";
  import Icon from "../lib/icons.svelte";
  import { refreshRuntime, runtimeStore } from "../lib/runtime-status.svelte.ts";
  import StudioPage from "../lib/pages/Studio.svelte";
  import SettingsPage from "../lib/pages/Settings.svelte";
  import YoutubePage from "../lib/pages/Youtube.svelte";
  import EditorPage from "../lib/pages/Editor.svelte";
  import ProjectPage from "../lib/pages/Project.svelte";

  // Browser preview (plain tab served by the adapter-node/dev server) has no
  // Tauri backend — file dialogs, crawl, analysis and every save action need
  // the desktop shell. Show a persistent banner so dead buttons explain themselves.
  const browserPreview = typeof window !== "undefined" && !isTauri();

  type PageKey = "studio" | "editor" | "youtube" | "settings" | "project";
  function pageKey(id: string): PageKey {
    if (id.startsWith("/projects")) return "project";
    if (id.startsWith("/editor")) return "editor";
    if (id.startsWith("/youtube")) return "youtube";
    if (id.startsWith("/settings")) return "settings";
    return "studio";
  }
  const titles: Record<PageKey, string> = {
    studio: "Local-first AI Video Studio",
    editor: "Editor · Local-first AI Video Studio",
    youtube: "YouTube Studio · Local-first AI Video Studio",
    settings: "Settings · Local-first AI Video Studio",
    project: "Project · Local-first AI Video Studio",
  };

  // Keep-alive pages: once visited, a page stays mounted and is shown/hidden
  // with CSS, so switching never re-runs onMount work — instant nav on top of
  // the CSR shell, and page state (search results, timeline) survives switching.
  const initialKey = pageKey(page.route?.id ?? "");
  let visited = $state<Record<PageKey, boolean>>({ studio: false, editor: false, youtube: false, settings: false, project: false, [initialKey]: true });
  const activePage = $derived(pageKey(page.route?.id ?? ""));
  // Write-only-when-missing: assigning a fresh object here would make this
  // effect depend on itself forever and starve every later state update.
  $effect(() => { if (!visited[activePage]) visited[activePage] = true; });

  const items = [
    { href: "/", id: "studio", label: "Studio" },
    { href: "/editor/demo", id: "editor", label: "Editor" },
    { href: "/youtube", id: "youtube", label: "YouTube Studio" },
    { href: "/settings", id: "settings", label: "Settings" },
  ];
  function isOn(item: { id: string }): boolean {
    if (item.id === "studio") return activePage === "studio" || activePage === "project";
    return activePage === item.id;
  }

  // Crash report from the previous run (Rust panic hook → panic.log → last_panic).
  let crashLog = $state("");

  async function minimize() { if (isTauri()) await getCurrentWindow().minimize().catch(() => {}); }
  async function toggleMax() { if (isTauri()) await getCurrentWindow().toggleMaximize().catch(() => {}); }
  async function closeWindow() { if (isTauri()) await getCurrentWindow().close().catch(() => {}); }

  onMount(() => {
    // Block the browser's default file-drop navigation once for the whole app
    // (the old per-Studio-mount window listeners leaked on every page switch).
    const block = (event: Event) => event.preventDefault();
    window.addEventListener("dragover", block);
    window.addEventListener("drop", block);
    void refreshRuntime();
    let poll: ReturnType<typeof setInterval> | undefined;
    if (isTauri()) {
      void invoke<string | null>("last_panic").then((text) => { crashLog = text ?? ""; }).catch(() => {});
      // runtime_status reports not-ready while the first-run model copy runs in
      // the background — poll a few times, then stop so a failure message stays
      // visible instead of being overwritten by endless retries.
      let attempts = 0;
      poll = setInterval(() => {
        if (runtimeStore.ready || attempts >= 5) { clearInterval(poll); return; }
        attempts += 1;
        void refreshRuntime(true);
      }, 3000);
    }
    return () => {
      window.removeEventListener("dragover", block);
      window.removeEventListener("drop", block);
      if (poll) clearInterval(poll);
    };
  });
</script>

<svelte:head><title>{titles[activePage]}</title><meta name="description" content="Turn long videos into short clips — 100% on-device." /></svelte:head>

<div class="shell">
  {#if isTauri()}
    <div class="titlebar" data-tauri-drag-region>
      <span class="titlebar-title">Local-first AI Video Studio</span>
      <div class="titlebar-controls">
        <button class="titlebar-btn" aria-label="Minimize" title="Minimize" onclick={minimize}><Icon name="min" size={14} /></button>
        <button class="titlebar-btn" aria-label="Maximize" title="Maximize" onclick={toggleMax}><Icon name="max" size={12} /></button>
        <button class="titlebar-btn titlebar-close" aria-label="Close" title="Close" onclick={closeWindow}><Icon name="x" size={14} /></button>
      </div>
    </div>
  {/if}
  {#if crashLog}
    <div class="crash-banner" role="alert">
      <div class="crash-body">
        <strong>App restarted after an unexpected error.</strong>
        <span class="note" style="margin:0;">Details from the previous run — please share this with support.</span>
        <pre class="crash-log">{crashLog}</pre>
      </div>
      <button class="btn btn-sm btn-quiet" onclick={() => crashLog = ""}>Dismiss</button>
    </div>
  {/if}
  {#if browserPreview}
    <div class="preview-banner" role="note"><Icon name="warning" size={16} /><span>Browser preview — crawl, file dialogs, analysis and every <strong>save</strong> action need the desktop app. Open the installed app for full functionality.</span></div>
  {/if}
  <div class="app" class:with-banner={browserPreview}>
    <nav class="rail" aria-label="Primary">
      <span class="rail-mark" title="Local-first AI Video Studio">LF</span>
      {#each items as item}
        <a class="rail-link" class:on={isOn(item)} href={item.href} title={item.label} aria-label={item.label}>
          <Icon name={item.id === "youtube" ? "youtube" : item.id === "editor" ? "timeline" : item.id === "settings" ? "settings" : "studio"} />
        </a>
      {/each}
      <div class="rail-foot">
        <span class="rail-dot" class:ok={runtimeStore.ready} title={runtimeStore.ready ? "Local engine ready" : "Engine incomplete"}></span>
        <span class="rail-tip">local</span>
      </div>
    </nav>
    <div class="frame">
      {#if visited.studio}<div class="page" class:off={activePage !== "studio"}><StudioPage active={activePage === "studio"} /></div>{/if}
      {#if visited.editor}<div class="page" class:off={activePage !== "editor"}><EditorPage active={activePage === "editor"} /></div>{/if}
      {#if visited.youtube}<div class="page" class:off={activePage !== "youtube"}><YoutubePage /></div>{/if}
      {#if visited.settings}<div class="page" class:off={activePage !== "settings"}><SettingsPage active={activePage === "settings"} /></div>{/if}
      {#if visited.project}<div class="page" class:off={activePage !== "project"}><ProjectPage /></div>{/if}
    </div>
  </div>
</div>
