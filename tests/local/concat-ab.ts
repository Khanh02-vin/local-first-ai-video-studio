/// A/B benchmark for the render join step (ADR-001's "one final renderer" — this
/// benchmark measures, it does not change anything).
///
/// The engine joins segments with the concat *filter* plus a full re-encode
/// (-filter_complex concat=n=N:v=1:a=1 -c:v libx264 …). The candidate is the concat
/// *demuxer* with -c copy: one less full encode (~30-50% of render wall time and one
/// less crf22 generation), but exactly the desync risk the production audit flagged —
/// each segment's AAC priming samples ride along in a stream copy, so the question is
/// whether audio-to-video alignment drifts across segment boundaries and whether it
/// *accumulates* with segment count.
///
/// Method: build three segments with the engine's exact encoder settings from a
/// source whose audio carries tones at known times, then join them both ways and
/// locate each tone in the outputs (silencedetect). Expected tone position is
/// exact by construction, so per-tone drift is measurable to the sample.
///
/// Verdict gates (printed at the end, decide GO/NO-GO for ADR-007):
///   G1  both outputs decode with zero errors (-v error)
///   G2  B's video/audio duration within 2 frames / 40 ms of A's
///   G3  B's tone onset drift vs expected ≤ 50 ms per tone AND no growth across
///       boundaries (the accumulation is what makes desync audible)
///   G4  no "Non-monotonous DTS" or decode warnings in B at -v warning
/// SSIM(A, B) and wall times are reported for context, not gated.
///
/// Run: npm run bench:concat-ab
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

const exec = promisify(execFile);
const ffmpeg = process.env.FFMPEG_PATH ?? "ffmpeg";
const ffprobe = process.env.FFPROBE_PATH ?? "ffprobe";

type Stream = { codec_type?: string; duration?: string; nb_read_frames?: string; sample_rate?: string };
type Probe = { streams?: Stream[]; format?: { duration?: string } };

async function run(cmd: string, args: string[], allowFailure = false): Promise<{ stdout: string; stderr: string }> {
  try {
    return await exec(cmd, args, { maxBuffer: 16 * 1024 * 1024 });
  } catch (error) {
    const err = error as { stdout?: string; stderr?: string; message: string };
    if (allowFailure) return { stdout: err.stdout ?? "", stderr: err.stderr ?? err.message };
    throw new Error(`${cmd} failed: ${err.stderr?.slice(-400) ?? err.message}`);
  }
}

async function probe(path: string): Promise<Probe> {
  const { stdout } = await run(ffprobe, ["-v", "error", "-count_frames", "-show_entries", "stream=codec_type,duration,nb_read_frames,sample_rate:format=duration", "-of", "json", path]);
  return JSON.parse(stdout) as Probe;
}

function stream(probeData: Probe, kind: "video" | "audio"): Stream | undefined {
  return probeData.streams?.find((s) => s.codec_type === kind);
}

/** Locates tone onsets: the output is tones surrounded by silence, so every
 *  silence_end in silencedetect is a tone start. Returns seconds, sorted. */
async function toneOnsets(path: string): Promise<number[]> {
  const { stderr } = await run(ffmpeg, ["-i", path, "-af", "silencedetect=noise=-35dB:d=1.2", "-f", "null", "-"], true);
  const onsets: number[] = [];
  for (const match of stderr.matchAll(/silence_end:\s*([0-9.]+)/g)) {
    onsets.push(Number(match[1]));
  }
  return onsets.sort((a, b) => a - b);
}

async function decodeErrors(path: string, logLevel: "error" | "warning"): Promise<{ errors: string[]; dtsWarnings: number }> {
  const { stderr } = await run(ffmpeg, [`-v`, logLevel, "-i", path, "-f", "null", "-"], true);
  const errors = stderr.split("\n").filter((line) => /error|invalid/i.test(line) && !/^\s*$/.test(line));
  const dtsWarnings = (stderr.match(/Non-monotonous DTS/g) ?? []).length;
  return { errors, dtsWarnings };
}

const root = await mkdtemp(join(tmpdir(), "concat-ab-"));
try {
  // --- Source: 60s 1280x720@30, audio with three tones at known times. Segments
  // cut [0..10], [15..23], [30..36], so expected output positions are 5.0, 15.0,
  // 20.0 and the expected total is 24.0s.
  const source = join(root, "source.mp4");
  const tone = (at: number, freq: number) =>
    `if(between(t,${at},${at + 2}),0.5*sin(${freq}*2*PI*t),0.01*sin(220*2*PI*t))`;
  await run(ffmpeg, [
    "-y", "-v", "error",
    "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=60",
    "-f", "lavfi", "-i", `aevalsrc='${tone(5, 440)}|${tone(20, 880)}|${tone(32, 1320)}':s=44100:d=60`,
    "-shortest", "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "128k",
    source,
  ]);

  // --- Segments with the engine's exact encoder settings (engine.rs: crop/scale per
  // aspect ratio, libx264 fast crf22 yuv420p -r 30, aac 128k; one segment carries a
  // drawtext hook so the batch covers heterogeneous filter graphs).
  const ranges = [
    { start: 0, dur: 10 },
    { start: 15, dur: 8, hook: true },
    { start: 30, dur: 6 },
  ];
  const segments: string[] = [];
  for (const [index, range] of ranges.entries()) {
    const segment = join(root, `segment-${index}.mp4`);
    const filters = [
      "crop=ih*9/16:ih:(iw-ih*9/16)/2:0",
      "scale=1080:1920",
      "setsar=1",
      ...(range.hook ? ["drawtext=text='HOOK':x=(w-text_w)/2:y=h*0.12:fontsize=h/18:fontcolor=white:borderw=3"] : []),
    ];
    await run(ffmpeg, [
      "-y", "-v", "error",
      "-ss", range.start.toFixed(3), "-t", range.dur.toFixed(3), "-i", source,
      "-vf", filters.join(","),
      "-map", "0:v:0", "-map", "0:a:0",
      "-c:v", "libx264", "-preset", "fast", "-crf", "22", "-pix_fmt", "yuv420p", "-r", "30",
      "-c:a", "aac", "-b:a", "128k",
      segment,
    ]);
    segments.push(segment);
  }
  const expectedTotal = ranges.reduce((sum, r) => sum + r.dur, 0);
  const expectedTones = [5.0, 10 + (20 - 15), 18 + (32 - 30)];

  // --- Path A: the engine's current join (concat filter + full re-encode). ---
  const pathA = join(root, "joined-filter.mp4");
  {
    const started = process.hrtime.bigint();
    let graph = "";
    for (let i = 0; i < segments.length; i++) graph += `[${i}:v:0][${i}:a:0]`;
    graph += `concat=n=${segments.length}:v=1:a=1[v0][aout];[v0]setsar=1[vout]`;
    await run(ffmpeg, [
      "-y", "-nostdin", "-hide_banner", "-loglevel", "error",
      ...segments.flatMap((s) => ["-i", s]),
      "-filter_complex", graph,
      "-map", "[vout]", "-map", "[aout]",
      "-c:v", "libx264", "-preset", "fast", "-crf", "22", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
      "-c:a", "aac", "-b:a", "128k",
      pathA,
    ]);
    process.env.__A_MS = String(Number(process.hrtime.bigint() - started) / 1e6);
  }

  // --- Path B: concat demuxer with -c copy (the candidate). ---
  const pathB = join(root, "joined-copy.mp4");
  {
    const listFile = join(root, "list.txt");
    await writeFile(listFile, segments.map((s) => `file '${s.replace(/'/g, "'\\''")}'`).join("\n") + "\n");
    const started = process.hrtime.bigint();
    await run(ffmpeg, [
      "-y", "-nostdin", "-hide_banner", "-loglevel", "warning",
      "-f", "concat", "-safe", "0", "-i", listFile,
      "-c", "copy", "-movflags", "+faststart",
      pathB,
    ]);
    process.env.__B_MS = String(Number(process.hrtime.bigint() - started) / 1e6);
  }

  // --- Measurements ---
  const probeA = await probe(pathA);
  const probeB = await probe(pathB);
  const onsetsA = await toneOnsets(pathA);
  const onsetsB = await toneOnsets(pathB);
  const decodeA = await decodeErrors(pathA, "error");
  const decodeBError = await decodeErrors(pathB, "error");
  const decodeBWarn = await decodeErrors(pathB, "warning");
  const ssimRun = await run(ffmpeg, ["-i", pathA, "-i", pathB, "-filter_complex", "[0:v][1:v]ssim", "-f", "null", "-"], true);
  const ssim = Number(ssimRun.stderr.match(/All:([0-9.]+)/)?.[1]);

  const durOf = (probeData: Probe, kind: "video" | "audio") => Number(stream(probeData, kind)?.duration ?? 0);
  const framesOf = (probeData: Probe) => Number(stream(probeData, "video")?.nb_read_frames ?? 0);

  const drift = (onsets: number[]) => {
    if (onsets.length < expectedTones.length) return Number.POSITIVE_INFINITY;
    return Math.max(...expectedTones.map((expected, i) => Math.abs(onsets[i] - expected) * 1000));
  };
  const driftA = drift(onsetsA);
  const driftB = drift(onsetsB);

  const gates = [
    { name: "G1 both decode clean (-v error)", pass: decodeA.errors.length === 0 && decodeBError.errors.length === 0 },
    { name: "G2 B duration within 2 frames (video) / 40ms (audio) of A",
      pass: Math.abs(durOf(probeB, "video") - durOf(probeA, "video")) <= 2 / 30 + 0.01
        && Math.abs(durOf(probeB, "audio") - durOf(probeA, "audio")) <= 0.04 },
    { name: "G3 B tone drift ≤ 50ms and not worse than A by >20ms",
      pass: driftB <= 50 && driftB - driftA <= 20 },
    { name: "G4 no Non-monotonous DTS in B", pass: decodeBWarn.dtsWarnings === 0 },
  ];
  const verdict = gates.every((g) => g.pass) ? "GO" : "NO-GO";

  const report = {
    benchmark: "concat-join-ab",
    segments: segments.length,
    expectedTotalSeconds: expectedTotal,
    expectedToneOnsets: expectedTones,
    filterConcat: {
      wallMs: Number(process.env.__A_MS),
      videoSeconds: durOf(probeA, "video"),
      audioSeconds: durOf(probeA, "audio"),
      videoFrames: framesOf(probeA),
      toneOnsets: onsetsA,
      toneDriftMs: driftA,
      decodeErrors: decodeA.errors,
    },
    demuxerCopy: {
      wallMs: Number(process.env.__B_MS),
      videoSeconds: durOf(probeB, "video"),
      audioSeconds: durOf(probeB, "audio"),
      videoFrames: framesOf(probeB),
      toneOnsets: onsetsB,
      toneDriftMs: driftB,
      decodeErrors: decodeBError.errors,
      dtsWarnings: decodeBWarn.dtsWarnings,
    },
    audioSampleRate: stream(probeA, "audio")?.sample_rate,
    ssimFilterVsCopy: Number.isFinite(ssim) ? ssim : null,
    gates,
    verdict,
  };
  console.log(JSON.stringify(report, null, 2));
  if (verdict === "NO-GO") process.exitCode = 1;
} finally {
  await rm(root, { recursive: true, force: true });
}
