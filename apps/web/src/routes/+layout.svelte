<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { page } from "$app/state";
  import Icon from "../lib/icons.svelte";

  // Browser preview (plain tab served by the adapter-node/dev server) has no
  // Tauri backend — file dialogs, crawl, analysis and every save action need
  // the desktop shell. Show a persistent banner so dead buttons explain themselves.
  const browserPreview = typeof window !== "undefined" && !isTauri();

  let runtimeReady = $state(false);

  onMount(() => {
    // Tauri-only command; the desktop shell injects window.__TAURI__. Outside
    // Tauri (adapter-node server in a plain browser) the fetch simply fails
    // and the runtime dot stays "incomplete" — the RAG endpoints do not
    // depend on any of the Tauri commands.
    invoke<{ ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; modelReady: boolean }>("runtime_status")
      .then((r) => { runtimeReady = r.ffmpeg && r.ffprobe && r.node && r.whisper && r.modelReady; })
      .catch(() => { runtimeReady = false; });
  });

  const items = [
    { href: "/", id: "studio", label: "Studio" },
    { href: "/editor/demo", id: "editor", label: "Editor" },
    { href: "/youtube", id: "youtube", label: "YouTube Studio" },
    { href: "/settings", id: "settings", label: "Settings" },
  ];
  const current = $derived(page.route?.id ?? "");

  function isOn(item: { id: string }): boolean {
    if (item.id === "studio") return current === "/" || current.startsWith("/projects");
    if (current === "/") return false;
    return current.startsWith(`/${item.id}`);
  }
</script>

<svelte:head><title>Local-first AI Video Studio</title><meta name="description" content="Turn long videos into short clips — 100% on-device." /></svelte:head>

{#if browserPreview}
  <div class="preview-banner" role="note">⚠ Browser preview — crawl, file dialogs, analysis and every <strong>save</strong> action need the desktop app. Open the installed app for full functionality.</div>
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
      <span class="rail-dot" class:ok={runtimeReady} title={runtimeReady ? "Local engine ready" : "Engine incomplete"}></span>
      <span class="rail-tip">local</span>
    </div>
  </nav>
  <div class="frame"><slot /></div>
</div>