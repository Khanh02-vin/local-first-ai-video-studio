/// Runs a child process and kills it as soon as an abort check turns true.
///
/// The analysis worker signs its own death by setting a flag on SIGTERM; without this
/// helper the whisper/ffmpeg child it spawned keeps running to completion after the
/// worker exits, holding CPU/VRAM with nobody left to read its output.
import { spawn } from "node:child_process";

export function runAbortable(
  command: string,
  args: string[],
  { timeoutMs, abortLabel, shouldAbort }: { timeoutMs: number; abortLabel: string; shouldAbort?: () => boolean },
): Promise<void> {
  return new Promise((resolve, reject) => {
    // The app ships its own Python venv (whisper, yt-dlp). Leaving the caller's
    // PYTHON* variables in place makes those interpreters pick up the venv's
    // site-packages and fail in confusing ways, so strip them before spawning.
    const env = { ...process.env };
    delete env.PYTHONHOME;
    delete env.PYTHONPATH;
    delete env.PYTHONSTARTUP;
    delete env.PYTHONEXECUTABLE;
    const child = spawn(command, args, { stdio: ["ignore", "ignore", "pipe"], env });
    let error = "";
    let settled = false;
    const stopTimer = () => { if (timer) clearTimeout(timer); if (abortTimer) clearInterval(abortTimer); };
    let timer: ReturnType<typeof setTimeout> | undefined;
    let abortTimer: ReturnType<typeof setInterval> | undefined;
    const finish = (failure?: Error) => { if (settled) return; settled = true; stopTimer(); failure ? reject(failure) : resolve(); };
    timer = setTimeout(() => { child.kill("SIGKILL"); finish(new Error("CHILD_TIMEOUT")); }, timeoutMs);
    // 100ms matches the renderer's cancel poll: fast enough to feel instant, slow
    // enough that the check never becomes the dominant cost of a long transcode.
    if (shouldAbort) {
      abortTimer = setInterval(() => {
        if (!shouldAbort()) return;
        child.kill("SIGKILL");
        finish(new Error(abortLabel));
      }, 100);
    }
    child.stderr.on("data", (chunk: Buffer) => { error += chunk.toString(); });
    child.on("error", (cause) => finish(new Error(`CHILD_SPAWN_ERROR:${cause.message}`)));
    child.on("close", (code, signal) => {
      if (settled) return;
      if (shouldAbort?.()) { finish(new Error(abortLabel)); return; }
      if (code === 0) { finish(); return; }
      finish(new Error(code === null ? `CHILD_KILLED:${signal ?? "signal"}` : `CHILD_FAILED:${error.slice(-300)}`));
    });
  });
}
