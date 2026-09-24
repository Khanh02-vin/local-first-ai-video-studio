<script lang="ts">export let data: { id: string };</script>
<h1>Project {data?.id ?? "demo"}</h1><section class="card"><h2>Sources</h2><p>Upload a long video to generate highlight candidates.</p><a href={`/editor/${data?.id ?? "demo"}`}>Open editor</a></section>
