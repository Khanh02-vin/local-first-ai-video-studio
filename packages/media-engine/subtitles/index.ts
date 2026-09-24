export type SubtitleWord = { word: string; start: number; end: number };
export type SubtitleTranscript = { segments: Array<{ words: SubtitleWord[] }> };

function stamp(seconds: number): string {
  const safe = Math.max(0, seconds);
  const h = Math.floor(safe / 3600);
  const m = Math.floor((safe % 3600) / 60);
  const s = Math.floor(safe % 60);
  const cs = Math.floor((safe - Math.floor(safe)) * 100);
  return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(cs).padStart(2, "0")}`;
}

function escapeText(text: string): string { return text.replace(/[\\{}]/g, (char) => `\\${char}`).replace(/\r?\n/g, " "); }

export function generateAss(transcript: SubtitleTranscript, clipStart: number, clipEnd: number): string {
  if (clipStart < 0 || clipEnd <= clipStart) throw new Error("Invalid subtitle clip range");
  const words = transcript.segments.flatMap((segment) => segment.words).filter((word) => word.end > clipStart && word.start < clipEnd).map((word) => ({ ...word, start: Math.max(0, word.start - clipStart), end: Math.min(clipEnd - clipStart, word.end - clipStart) }));
  const lines: string[] = [];
  for (let i = 0; i < words.length; i += 4) {
    const group = words.slice(i, i + 4);
    lines.push(`Dialogue: 0,${stamp(group[0].start)},${stamp(group[group.length - 1].end)},Shorts,,0,0,0,,${escapeText(group.map((word) => word.word).join(" "))}`);
  }
  return `[Script Info]\nScriptType: v4.00+\nPlayResX: 1080\nPlayResY: 1920\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Shorts,Arial,70,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,1,0,1,6,0,2,40,40,650,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n${lines.join("\n")}\n`;
}
