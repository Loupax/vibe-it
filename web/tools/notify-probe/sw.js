// C-BELL probe: a service worker, so the page can call registration.showNotification. On a
// 'notify' message it shows one itself (a worker context: the renderer shim does not reach it, so
// with C-BELL it must fail and nothing may reach D-Bus) and answers with what happened.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
self.addEventListener('message', e => {
  const perm = typeof Notification === 'undefined' ? 'no Notification' : Notification.permission;
  e.waitUntil(self.registration.showNotification('worker title', {body: 'worker body', tag: 'worker-tag'})
    .then(() => 'resolved', err => 'reject ' + err)
    .then(r => e.source.postMessage({perm, show: r})));
});
