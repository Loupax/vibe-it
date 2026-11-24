# C-BELL spike: a stand-in desktop notification server (org.freedesktop.Notifications) for a
# PRIVATE session bus: run.sh-private.sh starts it under dbus-run-session, never on the user's bus.
# It logs every call, so a run shows whether Chromium's notification bridge reaches D-Bus.
# Usage (inside the private bus): python3 -I fake_notifyd.py LOG
import sys, dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib
LOG = open(sys.argv[1], 'a', buffering=1)
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
class N(dbus.service.Object):
    n = 0
    @dbus.service.method('org.freedesktop.Notifications', in_signature='', out_signature='as')
    def GetCapabilities(self):
        LOG.write('GetCapabilities\n')
        return ['body', 'actions', 'body-markup', 'body-hyperlinks', 'body-images', 'icon-static', 'persistence']
    @dbus.service.method('org.freedesktop.Notifications', in_signature='', out_signature='ssss')
    def GetServerInformation(self):
        LOG.write('GetServerInformation\n')
        return ('fake-notifyd', 'vibe-it', '0', '1.2')
    @dbus.service.method('org.freedesktop.Notifications', in_signature='susssasa{sv}i', out_signature='u')
    def Notify(self, app, rid, icon, summary, body, actions, hints, timeout):
        N.n += 1
        LOG.write('Notify app=%r replaces=%r summary=%r body=%r actions=%r hints=%r\n' % (
            str(app), int(rid), str(summary), str(body), [str(a) for a in actions], sorted(str(k) for k in hints)))
        return dbus.UInt32(N.n)
    @dbus.service.method('org.freedesktop.Notifications', in_signature='u', out_signature='')
    def CloseNotification(self, i):
        LOG.write('CloseNotification %d\n' % i)
name = dbus.service.BusName('org.freedesktop.Notifications', bus)
N(bus, '/org/freedesktop/Notifications')
LOG.write('fake-notifyd up on %s\n' % bus.get_unique_name())
GLib.MainLoop().run()
