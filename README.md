# Bridge

Send files and text between PCs, Android phones and between PCs and Android phones over your home network.
Connections are encrypted, nothing goes through the internet, and no account is needed.

- **PC app** (Windows, macOS, Linux): a window with a chat per device, drag-and-drop files, a tray icon,
  and a global hotkey that sends the clipboard to the phone.
- **Android app**: pairs by scanning a QR code on the PC, has a small chat, a file list, and a **Share** menu entry.
- **PC to PC**: PCs running Bridge on the same network can pair with each other and exchange messages and files.

**Bridge never connects to the internet.** It has no update check, no telemetry and no account. To update, download the
new version from the [releases page](../../releases) and install it over the old one.
<table>
  <tr>
    <p align="center">
<img width="415" height="317" alt="grafik" src="https://github.com/user-attachments/assets/f490a150-0ac8-4d26-97b0-4662b2f668bc" /> <img width="415" height="317" alt="image" src="https://github.com/user-attachments/assets/004b5dcd-5432-4233-b06e-1417e9a698e6" />
      </p>
      </tr>
</table>
<table>
  <tr>
    <p align="center">
<img width="415" height="317" alt="image" src="https://github.com/user-attachments/assets/1dbdc80c-2dd1-4119-ab79-c78109755a83" />
<img width="415" height="317" alt="Pasted image 2026-11-06 06-52-00" src="https://github.com/user-attachments/assets/7447284b-94bd-4d0d-b3fb-5d22b36eac52" />
      </p>
      </tr>
</table>
<p align="center">
<img width="210" height="440" alt="Screenshot_20261106-075855_Bridge" src="https://github.com/user-attachments/assets/cf9d56d3-4e8d-42be-9f90-fa14d6d5d8d5" />
<img width="210" height="440" alt="Screenshot_20261106-075803_Bridge" src="https://github.com/user-attachments/assets/601191fc-e5db-4965-b574-ff621073195a" />
</p>

## Install

Download from the [latest release](../../releases/latest).

| System | File | Notes |
|---|---|---|
| Windows 10/11 | `Bridge-x.y.z-windows-x64-setup.exe` | Installs for your user, no administrator needed. A `-portable.zip` is there too. |
| macOS 11+ | `Bridge-x.y.z-macos.dmg` | Intel and Apple Silicon. Drag Bridge to Applications. |
| Debian, Ubuntu, Mint | `bridge-app_x.y.z_amd64.deb` | `sudo apt install ./bridge-app_x.y.z_amd64.deb` |
| Arch Linux | `bridge-app-x.y.z-1-x86_64.pkg.tar.zst` | `sudo pacman -U bridge-app-*.pkg.tar.zst` |
| Other Linux | `Bridge-x.y.z-linux-x86_64.tar.gz` | Needs GTK 3, `libayatana-appindicator` and `xdotool`. |
| Android 10+ | `Bridge-x.y.z-android.apk` | Open it on the phone and allow installing from this source when Android asks. |

`SHA256SUMS` lists a checksum for every file. To check a download: `sha256sum -c SHA256SUMS --ignore-missing`
(Linux), `shasum -a 256 -c SHA256SUMS --ignore-missing` (macOS), or `Get-FileHash file` (Windows PowerShell).

### The first start

The installers are **not signed with a paid certificate**, so the systems ask for a confirmation once:

- **Windows:** SmartScreen says "Windows protected your PC". Click *More info*, then *Run anyway*.
- **macOS:** the first time, right-click Bridge in Applications, choose *Open*, then *Open* again. If macOS says the app is
  damaged, run `xattr -dr com.apple.quarantine /Applications/Bridge.app` once.
  Allow *Local Network* when macOS asks (Bridge needs it to find your devices). The hotkey needs *Accessibility* in System Settings.
- **Android:** install from a file manager or browser, and allow "Install unknown apps" for that app.
  The APK is signed with the project's key; later versions install over earlier ones.

If a firewall asks, allow Bridge on **private networks**. Bridge uses TCP ports 8443 (phones) and 8444 (other PCs),
and mDNS (UDP 5353) to find devices.

## Pair a phone

1. On the PC choose **Pair a device > Pair a phone**. A QR code appears.
2. On the phone open Bridge, choose **Pair a PC** and scan the code.

To add another PC or another phone later, tap the round **+** on the phone's home screen (phone to phone: both phones show a
six-digit code, which you compare). The guides for each app explain the rest:
[PC guide](desktop/README.md) and [Android guide](android/README.md).

The QR code contains the PC's address, a one-time code and the PC's certificate fingerprint. The phone only ever talks to
a server with exactly that certificate, so nobody else on the network can pose as your PC. The next section explains
what that means in detail.

## How it works and what protects it

### The short version

- Every PC runs a small server on your home network. Phones and other PCs connect **directly** to it. There is no cloud
  server, no relay, no account, and no connection to the internet.
- Every connection is **HTTPS**: ordinary web requests (JSON, file chunks) carried inside **TLS**. Bridge has no plain-HTTP
  port. Because the devices talk to each other directly, the TLS connection ends on your own devices, so nobody in between
  can read it.
- Nobody vouches for a PC's certificate. Instead each device remembers the **fingerprint** of the certificates it paired
  with and accepts nothing else.
- Bridge encrypts the connection, **not the files on disk** (see "What is and is not encrypted" below).

### The connections

| Port | Used by | TLS | How each side proves who it is |
|---|---|---|---|
| 8443/tcp | the Android app | server certificate only | The PC proves itself with its certificate, which the phone has pinned. The phone proves itself with a secret token. |
| 8444/tcp | other PCs | mutual TLS (both sides present a certificate) | Both sides prove they hold the key of their certificate. The fingerprint is checked against the list of paired PCs. |
| a port the phone picks (TCP) | other phones, and PCs talking to a phone | mutual TLS (both sides present a certificate) | Same as between PCs: each phone has its own certificate; the six digits shown on both screens while pairing are compared by you, and afterwards only that fingerprint is accepted. |
| 5353/udp | finding devices (mDNS) | none | Only announces the device's name, fingerprint, address and port. Can be switched off in Settings. |

Bridge also listens on one port on `127.0.0.1` only (the PC port plus one, normally 8445). It is not reachable from the
network and carries no data; a second start of Bridge uses it to ask the running copy to show its window.
Bridge does not open ports on your router (no UPnP). It listens on the addresses of your computer, so on a network you do
not trust (a hotel or café), keep the firewall closed for Bridge.

### What TLS is used here

- **Versions and algorithms.** TLS 1.3 when both sides support it (the PC app and Android 10 or newer do), otherwise TLS 1.2.
  Older versions are refused. Only key exchanges with forward secrecy (ephemeral Diffie-Hellman) and authenticated
  encryption (AES-GCM or ChaCha20-Poly1305) are offered, so recorded traffic stays unreadable even if a key leaks later.
  The TLS library on the PC is rustls, and Android's own TLS stack is used on the phone.
- **Each PC has its own identity.** On the first start it creates a self-signed certificate with an ECDSA P-256 key.
  The private key never leaves the PC. The **fingerprint** (SHA-256 of the certificate) is the PC's identity.
- **No certificate authority.** Bridge does not use the system's list of trusted authorities, and it does not check
  the certificate's host name or expiry date. It asks one question instead: is this exactly the certificate I paired with?
  A change of the PC's IP address therefore never breaks a pairing, and a PC that someone sets up under the same name
  and address is still refused. The handshake signature is verified as usual, so whoever answers must hold the matching
  private key. If a PC's data folder is deleted, it creates a new identity and has to be paired again.

### Pairing a phone

1. The QR code holds `https://<address>:8443/#pair=<code>&fp=<fingerprint>`. The part after `#` is never sent over the
   network; the app reads it from the code itself.
2. The code is 10 characters from an alphabet of 32 (50 bits), valid for 5 minutes, and works once. Five wrong codes in a
   row lock pairing for a minute and cancel the code that was shown.
3. The app connects with the fingerprint pinned: if the PC presents any other certificate, the connection is dropped
   before anything is sent. The code is exchanged for a random 256-bit **token**.
4. The PC stores only a hash of the token. The phone keeps the token and the fingerprint in the app's private storage
   (Android's app sandbox; backups of the app are switched off). The app refuses unencrypted connections altogether.
   From then on every request carries the token, inside TLS.

If you type the address instead of scanning, the phone does not know the fingerprint yet. It accepts the certificate it sees
on that first connection, pins it from then on, and shows its start so you can **compare it with the one in the PC's
Settings**. The one-time code still keeps strangers from pairing. But someone who can sit between the phone and the PC
during that one first connection could present a certificate of their own, which is exactly what the comparison catches.

For testing, the PC also serves a small page at `https://<address>:8443/` for a phone's browser. A browser cannot pin
a fingerprint; it shows a certificate warning that you accept once, and the page holds no data until you pair it with a code.
The Android app is the intended way.

### Pairing two PCs

PC-to-PC connections use mutual TLS: each PC presents its certificate and proves it holds the key. At the TLS level any
certificate is accepted (otherwise a new PC could never start pairing), and Bridge then checks the fingerprint itself. A PC
that is not paired can reach only the greeting (its name) and the pairing steps; every other request is refused.

Pairing is a **numeric comparison**, like Bluetooth:

1. The PC that starts sends a hash of a random value (a commitment). The other PC answers with a random value of its own.
2. Only then does the first PC reveal its value, which the other PC checks against the commitment.
3. Both PCs compute the same six-digit number from SHA-256 of both certificate fingerprints and both random values,
   and show it.
4. Each user checks that the numbers match and confirms. Only after both confirmed does each PC save the other's fingerprint.

A third PC in the middle would have to use its own certificates, so the fingerprints, and therefore the numbers, differ.
Because of the commitment it cannot pick its value after seeing the other, so a matching number is a one-in-a-million
chance per attempt, and every failed attempt is visible as two different numbers. A PC accepts at most 4 open requests
and 10 new ones per minute, and a request expires after 2 minutes.

### What is and is not encrypted

| | |
|---|---|
| **Encrypted in transit** | Everything between your devices: message text, files, file names and sizes, transfer progress, pairing, tokens. |
| **Visible to others on the network** | That two devices are talking (addresses, ports, timing, roughly how much data moves). While discovery is on, each PC announces its name, fingerprint, address and port by mDNS. |
| **Not encrypted on disk** | Received files are ordinary files in your inbox folder. The chat history, the list of paired devices, the hashed tokens and the settings are files in the data folder, and so is the PC's private key (readable only by your user account on Linux and macOS; on Windows protected by the permissions of your user profile). On the phone, the messages are in the app's private storage, and received files are saved in the `Download/Bridge` folder, where other apps with access to your downloads can read them. |
| **Not provided** | Bridge has no password, PIN or app lock. Anyone who can read your data folder can copy the PC's identity. Use your system's disk encryption (BitLocker, FileVault, LUKS, Android's storage encryption) and a locked account if that matters. |

### You decide what arrives

A PC asks before it accepts a file from a phone or another PC, and the phone asks before it receives one. A file offer
can be given a time limit in Settings. Text from another PC is never put on your clipboard automatically. Files are
checked against a SHA-256 checksum, and an interrupted transfer continues where it stopped.

### What Bridge does not do

It does not protect against someone who already controls your PC or phone, malware on either, or an attacker who has
your data folder. It cannot hide that your devices talk to each other, and it does not work outside your home network.
It does not check whether you compared the numbers or the fingerprint; that step is yours.
The protocol is described in [docs/PROTOCOL.md](docs/PROTOCOL.md).

## Build from source

**PC app** (Rust, https://rustup.rs):

```
cd desktop
cargo run --release -p bridge-gui
```

Linux needs `libgtk-3-dev libayatana-appindicator3-dev libxdo-dev` (Debian/Ubuntu) or
`gtk3 libayatana-appindicator xdotool` (Arch). Windows needs the Visual Studio Build Tools (C++ workload).
Tests: `cargo test --workspace`.

**Android app:** open the `android` folder in Android Studio and press Run.

## Repository

| Folder | Contents |
|---|---|
| `desktop/` | Rust workspace: `core` (server, protocol, transfers), `gui` (the window), `cli` (console version for debugging) |
| `android/` | Kotlin / Jetpack Compose app |
| `packaging/` | Installer definitions: NSIS (Windows), disk image script (macOS), PKGBUILD (Arch), icons, desktop entry |
| `.github/workflows/` | `ci.yml` (tests on every push), `release.yml` (builds all installers for a version tag) |
| `docs/` | [Protocol](docs/PROTOCOL.md), [Releasing](docs/RELEASING.md) |

## Licence

[MIT](LICENSE)
