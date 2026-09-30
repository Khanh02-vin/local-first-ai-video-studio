<script lang="ts">
  import { page } from "$app/state";
  // Keep-alive mounted by +layout.svelte; the id comes from the URL so the
  // component works without SvelteKit page data.
  const id = $derived(page.params.id ?? "demo");
</script>

<h1>Project {id}</h1><section class="card"><h2>Sources</h2><p>Upload a long video to generate highlight candidates.</p><a href={`/editor/${id}`}>Open editor</a></section>
