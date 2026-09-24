# Desktop shell

Tauri 2 shell for the SvelteKit local-first app.

## Development

```bash
cd apps/web && npm run build
cd ../desktop && npm install
npm run tauri dev
```

Requires Rust/Cargo and platform Tauri prerequisites. The shell currently hosts the Svelte UI; local runner command integration is the next desktop task.

## Release

Do not claim a signed release until these environment values are supplied by the release owner:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- Platform code-signing/notarization credentials.

FFmpeg/model redistribution requires license and checksum review before bundling.
