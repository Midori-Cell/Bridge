# Bridge for Android

The phone app of [Bridge](../README.md): send text and files between your phone and your PC, over your home network.
It needs Android 10 or newer. The PC program is described in the [PC guide](../desktop/README.md).

## Install

Download `Bridge-x.y.z-android.apk` from the [releases page](../../../releases) on the phone and open it. Android asks you to
allow installing from that source (your browser or file manager); confirm once. Later versions install over earlier ones.

Bridge asks for the **camera** (to scan the pairing QR code) and, on Android 13 and newer, for **notifications** (to show a
transfer that is running). Nothing else.

## Pair with your PC

1. On the PC, open Bridge, click **Pair a device > Pair a phone**. A QR code appears.
2. On the phone, open Bridge and point the camera at the code. That is all.

To add another PC later, open *Settings > Pair another PC*. If the camera does not work, choose **Enter code instead** and type
the PC's address (shown in the PC's *Settings* as "Phone address") and the code. Without the QR code the phone cannot check the
PC's certificate on its own, so it then shows the start of the PC's fingerprint; compare it with the one in the PC's *Settings*
and tap *It matches*.

## Using the app

**Notes** is the home screen: the PC's name and connection status at the top, a small chat below, and the latest items.

- Type a note and tap **Send**. Tap **Clipboard** to paste what you copied into the box, and **Files** to choose files to send.
- **Long-press a message** for *Copy*, *Reply* and *Delete* (Delete removes it on the PC too). Long-press a file for *Open* or
  *Save*, and *Delete*.
- **Links** in messages open in your browser when you tap them.
- **Change** (next to the PC's name) switches between PCs if you paired more than one.

**Files** shows what is on its way, with progress and *Cancel*, and every file that went back and forth.

**Settings** has the phone's name, the paired PCs (tap one to make it the active one, or *Unpair*), *Pair another PC*, and
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

- Bridge follows the conversation only while the app is open. It does not receive messages in the background and shows no
  notification for them. A running file transfer continues with the screen off.
- The phone's name is taken from Android's device name. Changing it in Settings applies the next time you pair.
- Folders cannot be sent; share files, or zip a folder first.

## Build it yourself

Open the `android` folder in Android Studio, let Gradle sync, and run it on a phone.
