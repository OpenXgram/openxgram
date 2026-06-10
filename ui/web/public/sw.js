// OpenXgram GUI service worker — minimal, enables PWA installability.
// network-first passthrough; falls back to cache when offline.
const CACHE = "oxg-gui-v1";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (e) => {
  const req = e.request;
  // only GET; never cache API (/v1/*) or cross-origin
  if (req.method !== "GET" || new URL(req.url).pathname.startsWith("/v1/")) return;
  e.respondWith(
    fetch(req)
      .then((res) => {
        const copy = res.clone();
        caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
        return res;
      })
      .catch(() => caches.match(req)),
  );
});
