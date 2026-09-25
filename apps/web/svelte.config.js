import adapterStatic from "@sveltejs/adapter-static";
import adapterNode from "@sveltejs/adapter-node";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// Two build targets from one source:
//   default          → adapter-static → web/build      (Tauri desktop frontendDist)
//   WEB_ADAPTER=node → adapter-node   → web/build-node (RAG sidecar server,
//                      bundled as a Tauri resource and spawned on 127.0.0.1
//                      so the static UI can fetch /api/rag/* cross-origin).
const useNode = process.env.WEB_ADAPTER === "node";

export default {
  preprocess: vitePreprocess(),
  kit: {
    adapter: useNode
      ? adapterNode({ out: "build-node" })
      : adapterStatic({ fallback: "index.html" }),
  },
};
