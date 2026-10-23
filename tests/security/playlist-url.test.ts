import assert from "node:assert/strict";

function validPlaylistUrl(value: string): boolean {
  try {
    const url = new URL(value);
    const lists = url.searchParams.getAll("list");
    return ["http:", "https:"].includes(url.protocol)
      && ["youtube.com", "www.youtube.com", "m.youtube.com"].includes(url.hostname)
      && !url.username && !url.password && !url.port && url.pathname === "/playlist"
      && lists.length === 1 && /^[A-Za-z0-9_-]{10,}$/.test(lists[0]);
  } catch { return false; }
}

assert.equal(validPlaylistUrl("https://www.youtube.com/playlist?list=PL1234567890"), true);
assert.equal(validPlaylistUrl("http://youtube.com/playlist?list=PL1234567890"), true);
for (const value of [
  "https://youtube.com.evil.test/playlist?list=PL1234567890",
  "https://user@youtube.com/playlist?list=PL1234567890",
  "ftp://youtube.com/playlist?list=PL1234567890",
  "https://youtube.com:444/playlist?list=PL1234567890",
  "https://youtube.com/playlist",
  "https://youtube.com/playlist?list=PL1234567890&list=PL0987654321",
  "https://youtu.be/abcdefghijk",
  "not a URL",
]) assert.equal(validPlaylistUrl(value), false, value);
console.log("playlist URL validation tests: ok");
