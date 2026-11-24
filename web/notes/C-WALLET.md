# C-WALLET: what asks KWallet at startup, and what `--password-store=basic` would change

Investigation only. Nothing in the host is changed. Probe: `tools/wallet-probe/` (run.sh, fake_wallet.py,
serve_cookie.py), run against the INSTALLED `~/.local/lib/web/current/web-host-cef` (CEF/Chromium of the
2026-11-24 install) with throwaway profiles, on a private D-Bus with no service activation
(`tools/notify-probe/private-bus.conf`), in the headless nested dwl. The user's session bus, wallet and
`~/.local/state/web` were never touched. The fake KWallet is in-memory and logs calls; the bus is also
watched with `dbus-monitor` (raw logs are not committed).

## 1. What asks, when, for what

Chromium's OSCrypt (`components/os_crypt`, Linux) encrypts cookie values (and saved logins, `Login Data`)
with an AES key. On Linux that key is kept in a "keyring"; which one is chosen from the desktop
environment. The host asks for it **at startup, about 0.1 s after the browser process starts, before any
page loads and before any cookie exists** (the key store is initialised, it is not the first cookie write).
Sequence seen on the private bus with `XDG_CURRENT_DESKTOP=KDE` (this user's value, `KDE_SESSION_VERSION=6`):

1. `NameHasOwner org.kde.kwalletd6` (is the wallet service there?). No owner: nothing more, see "fallback".
2. `org.kde.KWallet.isEnabled`, `networkWallet` (-> "kdewallet").
3. `openAsync(wallet="kdewallet", appid="Chromium")`: this is the call that makes kwalletd show its
   **password dialog** when the wallet is locked (what appeared in the nest during C-BELL: a fresh private
   kwalletd that nothing had unlocked).
4. `hasFolder("Chromium Keys")`, `createFolder`, then `writePassword(folder "Chromium Keys", key
   "Chromium Safe Storage")` on the first run, `readPassword` on later runs; `close`.

So yes: the "Chromium Safe Storage" key in the folder "Chromium Keys" of the default wallet. The appid,
folder and key are Chromium's product name, not "web": **any Chromium-based app that uses KWallet shares
the same entry** (a Chromium browser installed on the machine, other CEF/Electron apps that did not rename
it). Nothing else in the host's startup talks to a wallet: the other D-Bus lookups at start
(`NameHasOwner` for `org.freedesktop.portal.Desktop`, `Notifications`, screensavers, `org.gtk.Settings`,
`org.gtk.vfs.Daemon`, `org.a11y.Bus`) are not secrets-related.

Backend selection (Chromium `components/os_crypt/sync/key_storage_util_linux.cc`, no `--password-store`
switch): desktop from `XDG_CURRENT_DESKTOP`; KDE with `KDE_SESSION_VERSION=6` selects `kwallet6`;
GNOME-like desktops select `gnome-libsecret`; anything else, and the fallback below, `basic`.
Measured with `XDG_CURRENT_DESKTOP=GNOME`: the host calls `org.freedesktop.Secret.Service.ReadAlias` on
`org.freedesktop.secrets` (libsecret) and never touches kwalletd.

## 2. What our installed web does today on the user's real desktop (reasoning + private-bus evidence)

The user's session is KDE 6 (`XDG_CURRENT_DESKTOP=KDE`, `KDE_SESSION_VERSION=6`); the host inherits the
environment and nothing overrides it. So Chromium picks `kwallet6`, and the steps of section 1 happen on
the real bus at every start. Cookies under `~/.local/state/web/...` are therefore `v11` (key from the
wallet): that is what the same host binary wrote for a KDE session in the probe (`t2`: prefix `v11`, 67
bytes for a 23-byte value). I did not look at the real files. Consequences:

- Whether the user ever sees a dialog depends on their wallet: with "kdewallet" unlocked at login (KDE's
  PAM auto-unlock, wallet password = login password) it is silent; otherwise a password prompt appears at
  **every start of web** until the wallet is opened. The key request is asynchronous, so the page load is
  not blocked in the normal case (in C-BELL's first runs, with the default bus config that activated the
  real portal and wallet daemons, the first page load did hang; the no-activation bus avoids it).
- Cost otherwise: about 5 D-Bus round trips.
- A wallet is not required (**fallback**): with no kwalletd on the bus (probe `t1c`, KDE) the host neither
  fails nor hangs. Chromium falls back to `basic` and writes `v10` cookies.

## 3. Options and their effect (measured in the probe, same host binary, throwaway profiles)

| Option | D-Bus traffic | Stored cookie | Notes |
|---|---|---|---|
| default on KDE (`kwallet6`) | the kwalletd6 calls above; may prompt | `v11`, key in KWallet | key shared with other Chromium apps. Protects against a copy of the profile dir (the key is not in it), not against a process running as the user (it can ask the wallet) |
| `--password-store=basic` | none at all (no kwallet lookup, no prompt) | `v10`, AES with Chromium's hard-coded key | effectively unencrypted: anyone who can read the files can decrypt them |
| `--password-store=gnome-libsecret` | `org.freedesktop.secrets` (`ReadAlias`, ...); on KDE served by ksecretd | `v11`, key in the Secret Service | same guarantee as kwallet. Measured only to the point of the first call (fake does not answer); ksecretd on the real bus not tried |
| `--password-store=kwallet6` explicitly | as default | `v11` | only useful where the desktop is not detected as KDE (started from a non-KDE session, stripped environment) |
| no wallet reachable (silent fallback) | `NameHasOwner` only | `v10` | what happens today if kwalletd is absent |
| a profile-local key | not offered | | Chromium and CEF have no such switch or setting; it would mean patching OSCrypt in the CEF build (`tools/cef-build`): not worth it |

What `basic` costs: the cookie jar and saved passwords are readable by whoever can read the profile dir,
i.e. the user and anything running as the user, and the latter can already ask the wallet. What it buys:
no wallet prompt, no coupling to the desktop's wallet state, no shared "Chromium Safe Storage" entry,
identical behaviour for agents, headless runs, other desktops and SSH. The protection the wallet adds on a
single-user desktop is narrow: a leaked/backed-up/rsynced copy of the profile dir cannot be read.

## 4. Backend changes and existing cookies (measured)

Copies of one throwaway profile, a cookie written under one backend, then started under another; the local
server logs the `Cookie` header of the first request (stored cookie decrypted = sent):

| Written under | Started under | Cookie sent | Result |
|---|---|---|---|
| `basic` (`v10`) | kwallet | yes | readable: Chromium always decrypts `v10` with the hard-coded key; it becomes `v11` when the site next sets it |
| kwallet (`v11`) | `basic` | **no** | unreadable, dropped: **logged out of every site** |
| kwallet (`v11`) | kwallet with a different key (wallet entry deleted or recreated) | **no** | same |

So switching from the KDE default to `basic` logs the user out everywhere once (nothing is migrated; saved
passwords in `Login Data` likewise). Switching back later does not log out again (`v10` stays readable).
The old `v11` rows are ignored, not an error. There is no in-place migration.

## 5. Recommendation

Use `--password-store=basic` for web. Reasons: web is a keyboard-driven single-user browser whose profile
dir is already private; the host also runs headless, nested and under agents, where a wallet prompt is
noise or a hazard (the C-BELL incident: a real KDE password dialog appeared in a test nest); today a locked
wallet means a prompt at every start, and the key lives in an entry shared with every other Chromium on the
machine. The cost is cookie-at-rest protection against someone who copies the profile directory; if the
user values that (backups leave the machine, say), keep the default and make sure "kdewallet" is unlocked at
login.

The decision is the user's. One-time effect of choosing `basic`: every site logs out once.

## 6. The exact change (not made)

`host-cef/src/main.rs`, `on_before_command_line_processing`, browser-process branch (next to
`disable-popup-blocking`):

```rust
// C-WALLET: no OS keyring for Chromium's cookie key (KWallet prompt, shared "Chromium Safe Storage").
cmd.append_switch_with_value(Some(&CefString::from("password-store")), Some(&CefString::from("basic")));
```

Better as an option, in the `--allow-background-traffic` pattern of `protocol.rs host_options`: a
`--password-store=<basic|kwallet6|gnome-libsecret>` host option, default `basic` if the user accepts the
recommendation (a switch given on the command line already reaches Chromium unchanged, so the shell
passing it through a `config.rs` constant works too). Plus a README line ("Cookies at rest"), a
DECISIONS.md entry, and a unit test that the switch is added (as `quiet_switches` is tested). Only the
browser process reads the key, so child processes need nothing. To keep today's behaviour and only stop the
prompt there is no switch; unlock "kdewallet" at login.

## Reproduce

```
web/tools/nest.sh                                   # prints wayland-N
WAYLAND_DISPLAY=wayland-N web/tools/wallet-probe/run.sh OUT TAG kwallet 40                    # KDE default, fake kwalletd6
WAYLAND_DISPLAY=wayland-N web/tools/wallet-probe/run.sh OUT TAG none 40 --password-store=basic
XDG_CURRENT_DESKTOP=GNOME WAYLAND_DISPLAY=wayland-N web/tools/wallet-probe/run.sh OUT TAG secrets 40
WEB_HOST_PROFILE=<copy of a profile> web/tools/wallet-probe/run.sh OUT TAG2 none 12 --password-store=basic   # backend change on a reused profile
```

40 s lets Chromium flush the cookie DB (30 s timer) before the scope is stopped.
