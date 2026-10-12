<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { studioState } from "../studio-state.svelte.ts";
  import type { Highlight } from "../../../../../packages/contracts/highlight.ts";
  import type { RenderPlan } from "../../../../../packages/contracts/render-plan.ts";

  type Project = {
    id: string;
    title: string;
    sourcePath: string;
    sourceName: string;
    sourceDuration: number;
    analysisJobId: string | null;
    transcript: unknown | null;
    highlights: Highlight[];
    selectedId: string | null;
    plans: Record<string, RenderPlan>;
    renders: Array<{ output: string; status: string; highlightId: string }>;
    updatedAt: string;
  };

  let projects = $state<Project[]>([]);
  let loading = $state(true);
  let error = $state("");

  onMount(async () => {
    if (!isTauri()) { loading = false; error = "Open projects in the desktop app."; return; }
    try { projects = await invoke<Project[]>("project_list"); }
    catch (cause) { error = `Could not load projects: ${String(cause)}`; }
    finally { loading = false; }
  });

  async function openProject(id: string) {
    try {
      const project = await invoke<Project>("project_get", { id });
      studioState.projectId = project.id;
      studioState.sourcePath = project.sourcePath;
      studioState.sourceName = project.sourceName;
      studioState.sourceDuration = project.sourceDuration;
      studioState.transcript = project.transcript as typeof studioState.transcript;
      studioState.highlights = project.highlights ?? [];
      studioState.selectedId = project.selectedId ?? project.highlights[0]?.id ?? "";
      studioState.plans = project.plans ?? {};
      studioState.plan = studioState.plans[studioState.selectedId] ?? null;
      await goto("/");
    } catch (cause) { error = `Could not open project: ${String(cause)}`; }
  }
</script>

<main class="bench">
  <section class="bench-center">
    <div class="stage">
      <div class="stage-head"><div class="stage-title"><span class="badge volt">projects</span><span class="stage-name">Your local projects</span></div><a class="btn btn-sm btn-quiet" href="/">New project</a></div>
      <div class="stage-body">
        {#if loading}<p class="hint">Loading projects…</p>
        {:else if error}<p class="message" role="status">{error}</p>
        {:else if projects.length}
          <div class="stack">
            {#each projects as project (project.id)}
              <article class="card project-row">
                <div class="preview-info">
                  <strong>{project.title || project.sourceName}</strong>
                  <span>{project.sourceName} · {project.highlights?.length ?? 0} clips · updated {new Date(Number(project.updatedAt) * 1000).toLocaleString()}</span>
                  <span class="note">{project.sourcePath}</span>
                </div>
                <button class="cta small" onclick={() => openProject(project.id)}>Open project</button>
              </article>
            {/each}
          </div>
        {:else}<div class="card"><p class="hint">No projects saved yet. Load a local video in Studio to create one.</p><a class="cta" href="/">Open Studio</a></div>{/if}
      </div>
    </div>
  </section>
</main>

<style>
  .project-row { display:flex; align-items:center; justify-content:space-between; gap:1rem; }
  .project-row .preview-info { min-width:0; }
  .project-row .note { overflow-wrap:anywhere; }
</style>
