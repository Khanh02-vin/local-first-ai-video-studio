<script lang="ts">
  // Stroke icons for the rail and chrome — inline SVG, zero dependencies.
  const paths: Record<string, string> = {
    studio: "M3 10.5 12 3l9 7.5M5 9v11h5v-5h4v5h5V9",
    timeline: "M3 3v18M7 12h4m3 0h7M7 7h3m0 0H7m0 0V4m7 13h-3m0 0h3m0 0v3",
    youtube: "M4 5h16l1.5 7-1.5 7H4L2.5 12 4 5zm6 4v6l6-3-6-3z",
    settings: "M12 8.5A3.5 3.5 0 1 0 12 15.5 3.5 3.5 0 0 0 12 8.5zm7.5 3.5c0-.4 0-.8-.1-1.1l1.7-1.3-1.7-2.9-2 .8a7.7 7.7 0 0 0-1.9-1.1L15.2 4h-3.4L11.5 6a7.7 7.7 0 0 0-1.9 1.1l-2-.8-1.7 2.9 1.7 1.3a8 8 0 0 0 0 2.2l-1.7 1.3 1.7 2.9 2-.8c.6.5 1.2.8 1.9 1.1l.3 2h3.4l.3-2a7.7 7.7 0 0 0 1.9-1.1l2 .8 1.7-2.9-1.7-1.3c.1-.3.1-.7.1-1.1z",
    film: "M4 4h16v16H4V4zm0 4h4M4 12h4m-4 4h4m12-4h-4m4-4h-4m4 8h-4M8 8h8v8H8V8z",
    folder: "M3 5h6l2 2h10v12H3V5z",
    clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zm0-13v5l3.5 2",
    check: "M4 12.5 9.5 18 20 6.5",
    x: "M6 6l12 12M18 6 6 18",
    plus: "M12 5v14M5 12h14",
    download: "M12 3v11m0 0 4-4m-4 4-4-4M4 19h16",
    link: "M9 15l6-6M7.5 12.5 5 15a3 3 0 0 0 4 4l2.5-2.5m5-5L19 9a3 3 0 0 0-4-4l-2.5 2.5",
    play: "M7 4.5v15l13-7.5-13-7.5z",
    pause: "M8 4v16M16 4v16",
    caption: "M3 5h18v14H3V5zm2 3h8v3H5V8zm0 5h6v2H5v-2z",
    refresh: "M20 12a8 8 0 1 1-2.3-5.7M20 3v4h-4",
    bolt: "M13 2 4 14h6l-1 8 9-12h-6l1-8z",
  };
  export let name: string;
  export let size = 18;
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.7"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name] ?? paths.studio} />
</svg>