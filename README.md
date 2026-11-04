# Bridge

Send text and files between your PC, your Android phone and your other PCs, over your home network.
Everything is encrypted, nothing goes through the internet, and no account is needed.

- **PC app** (Windows, macOS, Linux): a window with a chat per device, drag-and-drop files, a tray icon,
  and a global hotkey that sends the clipboard to the phone.
- **Android app**: pairs by scanning a QR code on the PC, has a small chat, a file list, and a **Share** menu entry.
- **PC to PC**: PCs running Bridge on the same network can pair with each other and exchange messages and files.

**Bridge never connects to the internet.** It has no update check, no telemetry and no account. To update, download the
new version from the [releases page](../../releases) and install it over the old one.

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
2. On the phone open Bridge; it starts with the camera ready. Scan the code.

To add another PC to the phone later, use *Settings > Pair another PC*. The guides for each app explain the rest:
[PC guide](desktop/README.md) and [Android guide](android/README.md).

The QR code contains the PC's address, a one-time code and the PC's certificate fingerprint. The phone only ever talks to
a server with exactly that certificate, so nobody else on the network can pose as your PC.

## How it works and what protects it

- **Local only.** Every PC runs a small server on your home network. The phone and other PCs talk to it directly. There is no
  cloud server, no account, and no connection to the internet. Messages, files and the list of paired devices are stored
  on your own devices only.
- **Encrypted.** All traffic uses TLS. Each PC creates its own self-signed certificate, and its SHA-256 fingerprint is
  that PC's identity. There is no certificate authority involved.
- **Phones pin the PC.** The QR code carries the PC's fingerprint, and the phone refuses to talk to anything that
  presents a different certificate. A one-time code in the QR code lets the phone pair; after that the phone uses a
  secret token, and the PC stores only a hash of it. Several wrong codes in a row lock pairing for a while.
- **PCs authenticate each other.** PC-to-PC traffic uses mutual TLS on its own port: both sides must present a certificate.
  Pairing shows a six-digit number on both screens (like Bluetooth "numeric comparison"), and each PC only saves the other
  after both users confirmed that the numbers match. Someone else on the network would have a different certificate and the
  two numbers would differ. A PC that has not paired can only answer its name and the pairing handshake.
- **You decide what arrives.** A PC asks before it accepts a file from a phone or another PC, and the phone asks before it
  receives one. Text from another PC is never put on your clipboard automatically. Files are checked against a SHA-256
  checksum, and an interrupted transfer continues where it stopped.
- **What can be seen on the network.** To find each other, PCs announce their name and certificate fingerprint on the
  local network (mDNS). This can be switched off in Settings. Everything else is inside the encrypted connection.

What Bridge does **not** do: it does not protect against someone who already controls your PC, your phone, or your Wi-Fi
router's settings, and it does not work outside your home network. If you pair by typing an address instead of scanning
the QR code, the phone cannot check the PC's certificate by itself, so it shows the fingerprint for you to compare.
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
