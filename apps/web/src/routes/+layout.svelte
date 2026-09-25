<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/state";
  import Icon from "../lib/icons.svelte";

  let runtimeReady = $state(false);

  onMount(() => {
    invoke<{ ffmpeg: boolean; ffprobe: boolean; node: boolean; whisper: boolean; modelReady: boolean }>("runtime_status")
      .then((r) => { runtimeReady = r.ffmpeg && r.ffprobe && r.node && r.whisper && r.modelReady; })
      .catch(() => { runtimeReady = false; });
  });

  const items = [
    { href: "/", id: "studio", label: "Studio" },
    { href: "/editor/demo", id: "editor", label: "Editor" },
    // YouTube Studio is rail-hidden until a RAG backend exists; the route stays in place for later re-enable.
    { href: "/youtube", id: "youtube", label: "YouTube Studio", hidden: true },
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

<div class="app">
  <nav class="rail" aria-label="Primary">
    <span class="rail-mark" title="Local-first AI Video Studio">LF</span>
    {#each items.filter((item) => !item.hidden) as item}
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