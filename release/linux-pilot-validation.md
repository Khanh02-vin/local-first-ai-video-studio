# Linux installer provenance and pilot validation

## Artifact

- File: `apps/desktop/src-tauri/target/release/bundle/deb/Local-first AI Video Studio_0.1.0_amd64.deb`
- Package / version / architecture: `local-first-ai-video-studio` / `0.1.0` / `amd64`
- SHA-256: `3b08a081628f297759dc9892afc4b5909d315e3aaf3eeb9c7e0637e5a618d585`
- Size: 325,291,044 bytes (dpkg metadata)
- Build host: Ubuntu 26.04.1 LTS, x86_64
- Source revision: `170389eda01c48e37a279626a1b685e885c85d8c` (`v0.1.0-63-g170389e-dirty`)
- Source tree was dirty at build time. Changes present: `apps/desktop/src-tauri/capabilities/default.json`, `apps/desktop/src-tauri/gen/schemas/capabilities.json`, `apps/web/src/app.css`, `apps/web/src/lib/icons.svelte`, `apps/web/src/lib/pages/Editor.svelte`, `apps/web/src/lib/pages/Settings.svelte`, `apps/web/src/lib/pages/Studio.svelte`, `apps/web/src/lib/pages/Youtube.svelte`, `apps/web/src/routes/+layout.svelte`.
- Declared runtime dependencies: `libwebkit2gtk-4.1-0`, `libgtk-3-0`.

## Validation status

- [x] Artifact built and SHA-256 recorded.
- [x] Pilot copy staged at `release/pilot-linux/Local-first AI Video Studio_0.1.0_amd64.deb`; matching `SHA256SUMS` is alongside it.
- [x] Debian control metadata inspected; package declares expected GTK/WebKitGTK dependencies.
- [x] Clean Ubuntu 24.04 package installation: `apt-get install` completed successfully in a fresh `ubuntu:24.04` container; dpkg reports the package installed and lists `/usr/bin/local-first-ai-video-studio` and its `.desktop` launcher. This verifies installation/dependency resolution only, not GUI launch.
- [ ] GUI launch and visible render on a clean desktop matching the customer OS/version.
- [ ] Three titlebar controls (minimize, maximize/restore, close).
- [ ] Drag window from blank titlebar area.
- [ ] Navigate between tabs while retaining state.
- [ ] Open native file picker and choose a test video.
- [ ] Run analysis using the installed package and confirm completion/output.
- [ ] While analysis is running, navigate tabs and verify UI responsiveness and job continuation; collect native FPS/CPU/memory measurements.

GUI behavior and native analysis through this `.deb` have **not** been verified. The clean container has no desktop display and cannot establish GUI behavior. Prior pipeline measurements ran the project scripts directly, not the packaged GUI app, and are not evidence for those unchecked items.

## Pilot checklist / tester report

Keep the pilot small and do not expand distribution until the checks above pass on the actual supported customer OS/version. Ask each tester to return:

- OS name/version, desktop environment, display protocol (Wayland/X11), CPU/GPU, and screen scaling.
- App version, installer filename, and SHA-256 (compare against the artifact section above before testing).
- Exact steps and outcome for app launch, titlebar buttons, titlebar drag, tab switching, file picker, and analysis.
- Whether tab changes during analysis affect progress, whether the app becomes unresponsive, and approximate analysis duration.
- App logs and relevant system logs around a failure, with private video paths or other sensitive material redacted.
- Screenshots/video of visible failures and exact timestamps; never send source videos unless separately approved.

Tester report template:

```text
OS/version:
Desktop environment + Wayland/X11:
Hardware + scaling:
App version:
Installer SHA-256:
Launch/render:
Minimize / maximize-restore / close:
Blank-titlebar drag:
Tab switching/state retention:
File picker:
Analysis result + duration:
Tab changes during analysis / responsiveness:
Logs attached (sensitive paths redacted):
Reproduction steps / screenshots:
```
