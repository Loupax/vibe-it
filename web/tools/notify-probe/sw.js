// C-BELL spike: an empty service worker, only so the page can call registration.showNotification.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
