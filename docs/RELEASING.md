# Releasing

Pushing a version tag builds every installer on GitHub and puts them on a **draft** release.
You look the draft over and press *Publish release*.

## One-time setup

### 1. Android signing key

Android only installs an update if it is signed with the same key as the installed app. **Keep this key and its passwords
safe and backed up outside GitHub. If you lose it, nobody can update; they would have to uninstall first.**

```
keytool -genkeypair -v -keystore bridge-release.jks -alias bridge -keyalg RSA -keysize 4096 -validity 36500
base64 -w0 bridge-release.jks > bridge-release.jks.base64     # on macOS: base64 -i bridge-release.jks
```

In the repository on GitHub open *Settings > Secrets and variables > Actions* and add four secrets:

| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | the contents of `bridge-release.jks.base64` |
| `ANDROID_KEYSTORE_PASSWORD` | the keystore password you chose |
| `ANDROID_KEY_ALIAS` | `bridge` |
| `ANDROID_KEY_PASSWORD` | the key password you chose |

Without the first secret the release still works, but without the Android app (the workflow prints a warning).

An APK built by Android Studio is signed with Android's debug key, so the first release APK cannot be installed over it.
Uninstall the debug build once (this removes its pairings; pair again).

### 2. Names to fill in

- `packaging/arch/PKGBUILD`: `_owner` (your GitHub name) and the `Maintainer` line.
- `desktop/gui/Cargo.toml`, `[package.metadata.deb]`: `maintainer`.
- `LICENSE`: the name after the year, if you do not want "Bridge contributors".

## Every release

1. Pick the version, for example `0.3.1` (numbers only: `x.y.z`).
2. Change it in **`desktop/Cargo.toml`** (`[workspace.package]`) and in **`packaging/arch/PKGBUILD`** (`pkgver`).
   The Android version needs no edit: the workflow takes it from the tag.
3. Run the tests: `cd desktop && cargo test --workspace`.
4. Commit and push to `main`, and wait for the *CI* run to turn green (it also compiles the Android app).
5. Tag and push:
   ```
   git tag v0.3.1
   git push origin v0.3.1
   ```
6. Open *Actions > Release* and wait (about 15 minutes). If a job fails, fix it, delete the tag
   (`git tag -d v0.3.1 && git push origin :refs/tags/v0.3.1`) and tag again.
7. Open the draft under *Releases*. Check that all of these are attached, edit the notes, and press *Publish release*:
   `-setup.exe`, `-portable.zip`, `.dmg`, `.deb`, `.pkg.tar.zst`, `-linux-x86_64.tar.gz`, `-android.apk`, `SHA256SUMS`.

## Worth testing on a real machine

The workflows build the installers, but only a person can see that they work:

- Windows: run the setup, start Bridge from the Start menu, pair a phone, uninstall (your data stays in `%APPDATA%`).
- macOS: open the `.dmg`, drag the app to Applications, right-click > Open, pair, check the *Local Network* question appears.
- Debian/Ubuntu: `sudo apt install ./bridge-app_*.deb`, find Bridge in the menu with its icon, tray icon visible.
- Arch: `sudo pacman -U bridge-app-*.pkg.tar.zst`.
- Android: install the APK, pair, send a file both ways.

## Optional: Arch User Repository

Publishing `bridge-app` on the AUR lets Arch users install with `yay -S bridge-app` and update with their normal system update:
copy `packaging/arch/PKGBUILD` into an AUR repository, run `updpkgsums` to fill in the checksum, then
`makepkg --printsrcinfo > .SRCINFO`, commit and push to the AUR.

## Not included

- No installer signing (Windows code-signing certificate, Apple notarization). The README explains the one-time warnings.
- No in-app updater, by design: Bridge never connects to the internet.
