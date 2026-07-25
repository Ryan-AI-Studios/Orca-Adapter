// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html (SPA mode)
export const ssr = false;
export const prerender = true;
