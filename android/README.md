# Bridge for Android

The phone app of [Bridge](../README.md): send text and files between your phone and your PC, or another phone, over your home network.
It needs Android 10 or newer. The PC program is described in the [PC guide](../desktop/README.md).

## Install

Download `Bridge-x.y.z-android.apk` from the [releases page](../../../releases) on the phone and open it. Android asks you to
allow installing from that source (your browser or file manager); confirm once. Later versions install over earlier ones.

Bridge asks for the **camera** (to scan the pairing QR code) and, on Android 13 and newer, for **notifications** (to show a
transfer that is running). Nothing else.

## Pair a device

Tap the round **+** next to the *Notes* title (or choose a device on the welcome screen) and pick **Pair a PC** or **Pair a phone**.

**A PC.** On the PC, open Bridge, click **Pair a device > Pair a phone**. A QR code appears. Point the phone's camera at it. If the
camera does not work, choose **Enter code instead** and type the PC's address (shown in the PC's *Settings* as "Phone address") and the
code. Without the QR code the phone cannot check the PC's certificate on its own, so it then shows the start of the PC's
fingerprint; compare it with the one in the PC's *Settings* and tap *It matches*.

**Another phone.** Both phones need Bridge 0.4 or newer and the same Wi-Fi. On the first phone choose *Pair a phone*; phones that are
visible nearby are listed (or enter an address). Tap the other phone: it asks *"NAME wants to pair"* and both phones show
the same six-digit code. If the codes are identical, tap *They match* on both. Only then is the connection saved. Each phone
has its own certificate, and from then on only that exact phone is trusted.

## Using the app

**Notes** is the home screen: the PC's name and connection status at the top, a small chat below, and the latest items.

- Type a note and tap **Send**. Tap **Clipboard** to paste what you copied into the box, and **Files** to choose files to send.
- **Long-press a message** for *Copy*, *Reply* and *Delete* (Delete removes it on the PC too). Long-press a file for *Open* or
  *Save*, and *Delete*.
- **Links** in messages open in your browser when you tap them.
- **Pictures** are shown in the chat itself. A picture that was offered shows as a blurred preview with *Tap to download*;
  nothing is downloaded until you tap it, then it is saved in Downloads/Bridge and shown sharp. Tap it again to see it large in the app:
  swipe for the other pictures, pinch or double-tap to zoom, drag to move, tap to hide the bar, Back to close. *Open in…*
  hands it to your gallery or another app. *Decline* (long-press) refuses it. A file you declined cannot be fetched from the chat afterwards.
- Under **FILES** below the chat, files that are waiting or on their way are listed first, then the last few; each line says
  *From* or *To* which device.
- The row of **device chips** under the title switches between your PCs and phones; every device has its own chat and files.
- **Clear chat** (right end of the status line) deletes the chat on this phone only. The phone keeps its own history, so
  *Clear chat* on the PC never removes it here, and the other way round.

**Files** shows what is on its way, with progress and *Cancel*, and every file that went back and forth.

**Settings** has the phone's name, **Visible to other phones** (lets nearby phones find this one while pairing),
**Receive while app is closed** (keeps a small notification so Bridge can receive in the background), this phone's address,
the paired devices with *Remove*,
**Notifications** (a message or file arriving while Bridge is not open: pop-up, sound, vibration and light, each with its own
switch; messages from a PC are only noticed while Bridge is open or *Receive while app is closed* is on), and
**Appearance**: *System* (follows Android, the default), *Off* (always light) or *On* (always dark).

### Sending files from other apps

In a gallery, file manager or browser choose **Share > Bridge**, pick the PC, and tap **Send**. Text can be shared too and
arrives as a note. The PC asks its user to accept each file before anything is sent.

### Receiving files

When your PC sends a file, Bridge asks **Receive a file?** with *Accept* and *Decline*. Accepted files are saved in
`Downloads/Bridge`; tap one in the chat to open it. A declined file stays in the chat with a *Save* button.

## If something does not work

- **"PC not reachable":** phone and PC need the same Wi-Fi network, and the PC program must be running. Guest Wi-Fi and
  "client isolation" block the connection. If the PC got a new address, the phone finds it again by itself.
- **"... unpaired this phone":** the PC removed the phone. Pair again with a new QR code.
- **The camera stays black:** allow the camera for Bridge in Android's app settings, or use *Enter code instead*.

## Limits

- Bridge follows the conversation with a PC only while the app is open. With **Receive while app is closed** on, a small
  notification keeps phone-to-phone receiving alive in the background. A running file transfer continues with the screen off.
- A phone-to-phone file that was only half received when the app is closed starts again from the beginning.
- The other phone must be reachable at the moment you send; if it is not, text and files wait ("Waiting to send") and go out
  when it is back.
- The phone's name is set in *Settings*; a PC or phone you paired earlier shows the new name the next time it connects.
- Folders cannot be sent; share files, or zip a folder first.

## Build it yourself

Open the `android` folder in Android Studio, let Gradle sync, and run it on a phone.

### Tests without Android

`jvm-tests/run.sh` runs the chat history and the phone-to-phone engine on a plain JVM (including a conversation with the real PC
program; build it first with `cargo build -p bridge-core --example peer_harness` in `desktop`).
