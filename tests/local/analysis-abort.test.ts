/// Verifies that asking the analysis worker to stop actually terminates the
/// transcriber it spawned, and that the normal path still returns a transcript.
///
/// Run: node --experimental-strip-types tests/local/analysis-abort.test.ts
import { strict as assert } from "node:assert/strict";
import { mkdtemp, writeFile, chmod, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { transcribeWithWhisper } from "../../adapters/local/whisper.ts";

const alive = (pid: number): boolean => {
  try { process.kill(pid, 0); return true; } catch { return false; }
};

// A stand-in for the real transcriber: writes its pid and, only if it survives,
// a "finished" marker plus whisper-shaped JSON in the directory it is told to use.
const fakeWhisper = async (dir: string, seconds: number): Promise<string> => {
  const path = join(dir, "fake-whisper.sh");
  await writeFile(path, `#!/usr/bin/env bash
set -u
echo $$ > "${join(dir, "pid")}"
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "--output_dir" ]; then out="$2"; shift 2; else shift; fi
done
sleep ${seconds}
[ -n "$out" ] && printf '%s' '{"language":"en","segments":[{"id":0,"text":"done talking","start":0,"end":2,"words":[{"word":"done","start":0,"end":1},{"word":"talking","start":1,"end":2}]}]}' > "${join(dir, "result.json")}"
cp "${join(dir, "result.json")}" "$out/result.json" 2>/dev/null || true
touch "${join(dir, "finished")}"
`);
  await chmod(path, 0o755);
  return path;
};

const dir = await mkdtemp(join(tmpdir(), "analysis-abort-"));
try {
  const audio = join(dir, "audio.wav");
  await writeFile(audio, "not really audio — the fake transcriber ignores it");

  // Success path: the transcriber is allowed to finish.
  {
    const command = await fakeWhisper(await mkdtemp(join(tmpdir(), "analysis-abort-ok-")), 0);
    const transcript = await transcribeWithWhisper(audio, "src-1", 10, { command, timeoutMs: 30_000, model: "tiny" });
    assert.equal(transcript.words.length, 2, "the shuffled word list should carry both words");
    assert.equal(transcript.segments.length, 1, "one segment should survive");
    assert.equal(transcript.language, "en");
  }

  // Cancel path: signal while the transcriber runs, and require it to die.
  {
    const child = await mkdtemp(join(tmpdir(), "analysis-abort-kill-"));
    const command = await fakeWhisper(child, 30);
    let stop = false;
    setTimeout(() => { stop = true; }, 300);
    await assert.rejects(
      transcribeWithWhisper(audio, "src-1", 10, { command, timeoutMs: 60_000, shouldAbort: () => stop }),
      /WHISPER_ABORTED/,
      "a cancel must surface as WHISPER_ABORTED, not a timeout or a success",
    );
    const pid = Number(await (await import("node:fs/promises")).readFile(join(child, "pid"), "utf8"));
    // The child is reaped by this process, so give the kernel a moment before
    // asserting the pid is gone rather than merely unreaped.
    await new Promise((resolve) => setTimeout(resolve, 300));
    assert.equal(alive(pid), false, `the transcriber (pid ${pid}) must not survive the cancel`);
    assert.equal(await (await import("node:fs/promises")).readFile(join(child, "result.json"), "utf8").catch(() => ""), "", "the killed transcriber must not have produced output");
    await rm(child, { recursive: true, force: true });
  }
} finally {
  await rm(dir, { recursive: true, force: true });
}
console.log("analysis abort tests: ok");
