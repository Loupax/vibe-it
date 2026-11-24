# C-WALLET: stand-ins for KWallet (org.kde.kwalletd6 / kwalletd5) and the Secret Service
# (org.freedesktop.secrets) for a PRIVATE session bus only (run.sh starts it under
# private-bus.conf, never on the user's bus). In-memory, logs every call; nothing is persisted.
# Usage (inside the private bus): python3 -I fake_wallet.py LOG kwallet|secrets|both|none
import sys, dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib
LOG = open(sys.argv[1], 'a', buffering=1)
MODE = sys.argv[2]
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
KW = {}   # (folder, key) -> bytes/str

def mk(name, path):
    class K(dbus.service.Object):
        I = 'org.kde.KWallet'
        def log(self, s): LOG.write('%s %s\n' % (name, s))
        @dbus.service.method(I, in_signature='', out_signature='b')
        def isEnabled(self): self.log('isEnabled'); return True
        @dbus.service.method(I, in_signature='', out_signature='s')
        def networkWallet(self): self.log('networkWallet'); return 'kdewallet'
        @dbus.service.method(I, in_signature='sxs', out_signature='i')
        def open(self, w, wid, app): self.log('open wallet=%s app=%s' % (w, app)); return 1
        @dbus.service.method(I, in_signature='sxsb', out_signature='i')
        def openAsync(self, w, wid, app, sess):
            self.log('openAsync wallet=%s app=%s' % (w, app))
            GLib.timeout_add(50, lambda: (self.walletAsyncOpened(1, 1), False)[1])
            return 1
        @dbus.service.signal(I, signature='ii')
        def walletAsyncOpened(self, tid, handle): self.log('-> walletAsyncOpened')
        @dbus.service.method(I, in_signature='iss', out_signature='b')
        def hasFolder(self, h, f, app): self.log('hasFolder %s %s' % (f, app)); return any(k[0] == f for k in KW)
        @dbus.service.method(I, in_signature='iss', out_signature='b')
        def createFolder(self, h, f, app): self.log('createFolder %s %s' % (f, app)); return True
        @dbus.service.method(I, in_signature='isss', out_signature='s')
        def readPassword(self, h, f, k, app): self.log('readPassword folder=%s key=%s' % (f, k)); return KW.get((f, k), '')
        @dbus.service.method(I, in_signature='issss', out_signature='i')
        def writePassword(self, h, f, k, v, app): self.log('writePassword folder=%s key=%s len=%d' % (f, k, len(v))); KW[(f, k)] = str(v); return 0
        @dbus.service.method(I, in_signature='ibs', out_signature='i')
        def close(self, h, force, app): self.log('close'); return 0
        @dbus.service.method(I, in_signature='bs', out_signature='i')
        def close2(self, force, w): return 0
    return K(bus, path), dbus.service.BusName(name, bus)

keep = []
if MODE in ('kwallet', 'both'):
    keep.append(mk('org.kde.kwalletd6', '/modules/kwalletd6'))
    keep.append(mk('org.kde.kwalletd5', '/modules/kwalletd5'))
if MODE in ('secrets', 'both'):
    # Only claims the name and answers nothing: dbus-monitor shows what libsecret/Chromium call.
    class S(dbus.service.Object): pass
    keep.append((S(bus, '/org/freedesktop/secrets'), dbus.service.BusName('org.freedesktop.secrets', bus)))
LOG.write('fake-wallet %s up\n' % MODE)
GLib.MainLoop().run()
