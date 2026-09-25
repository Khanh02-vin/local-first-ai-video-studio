// Builds the SvelteKit app with WEB_ADAPTER=node so svelte.config.js picks
// adapter-node and writes to build-node/ — the RAG sidecar that the Tauri
// desktop app spawns on 127.0.0.1. Leaves the default static build in
// build/ (Tauri frontendDist) untouched.
//
// A wrapper script instead of an inline `WEB_ADAPTER=node vite build` in
// package.json because `VAR=value cmd` does not run under cmd.exe on Windows,
// and the same script is invoked by tauri.conf beforeBuildCommand on all OSes.
process.env.WEB_ADAPTER = "node";
const { build } = await import("vite");
await build();
