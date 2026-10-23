<script lang="ts">
  import { goto } from "$app/navigation";

  async function openProjects() {
    await goto("/projects/local");
  }
</script>

<svelte:head><title>Projects · Local-first AI Video Studio</title></svelte:head>
<button class="btn btn-quiet" onclick={openProjects} aria-label="Open local projects">Open local projects</button>
