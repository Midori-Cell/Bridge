# Bridge for PC

The PC program of [Bridge](../README.md): a window for sending text and files to your phone and to other PCs on your home
network. Installers are on the [releases page](../../../releases); this page explains how to use the program.

## First start

Bridge asks where received files should be saved. The suggestion is a `Bridge` folder inside your Downloads folder.
Click **Choose folder...** to pick another one, then **Continue**. You can change it later in *Settings > Received files*.
New files go to the new folder; files you already received stay where they are.

If your firewall asks, allow Bridge on **private networks**. Bridge uses TCP port 8443 (phones), TCP port 8444
(other PCs) and UDP port 5353 (mDNS, how your devices find each other). Only one copy of Bridge can run at a time.

## Pair a phone

1. Click **Pair a device** (under the search box) and choose **Pair a phone**. A QR code appears.
2. In the Bridge app on the phone, scan the code. The phone appears in the sidebar under its own name (for example
   "Pixel 8"). **Rename** changes the name.

The code expires after a few minutes; a new one is shown each time. See the [Android guide](../android/README.md) for the phone side.

## Pair another PC

1. On both PCs click **Pair a device > Pair a PC**. Each PC lists the other under **Nearby**. If it does not appear,
   choose **Pair by address...** and enter the address shown in the other PC's *Settings*.
2. Click **Pair** on one PC. The other PC shows "wants to pair" and brings its window forward.
3. Both screens show the **same six-digit code**. If the codes are the same, click *Codes match* on both PCs.
   Only then are the PCs paired. If they differ, click *Cancel*: somebody may be in the way.

What another PC can see of you: your PC's name, nothing else. Not your phone, your history or your files.

## Using the window

**The sidebar** lists your paired phones and PCs with their last activity and a short preview. **Search** filters them by
name or last message. **Settings** is at the bottom.

**A conversation** shows your messages on the right in blue and theirs on the left. Every device has its own conversation: a phone only ever gets the messages and files that were sent between it and this PC, never those of another phone.

- **Send text:** type in the box and press *Send* (or Ctrl+Enter; the key is a setting). **Clipboard** (phones only) sends
  the text you copied.
- **Send files:** click **Files**, or drag files onto the window.
- **Send a screenshot:** copy a picture (for example with the Snipping Tool or Cmd+Shift+Ctrl+4), open a conversation and press
  Ctrl+V (Cmd+V on a Mac). It is sent as a PNG file.
- **Reply, copy, delete:** hover a message to see the buttons, or double-click it to reply. Delete removes the message on both devices.
- **Open a file** by clicking it. Hover it for **Show in folder** and a red **x** to delete it. Programs and scripts
  (`.exe`, `.sh`, `.bat` and so on) are never started from Bridge; clicking them shows their folder instead.
- **Links:** web addresses in messages are underlined; click one to open it in your browser.
- **Chat height:** drag the small grip under the chat. Double-click it to make the chat follow the window again.
- **Mark and copy text:** drag over the text of a message to mark it, then press Ctrl+C (Cmd+C on a Mac).
- **Message width:** hold Ctrl (Cmd on a Mac) and drag a message to the left or right. All your messages (or, if you drag
  an incoming one, all incoming messages) get that width. Without the key a drag marks text instead. *Settings > Message width > Reset* makes every message as wide as its text again.
- **Times and days:** every message shows its time at the bottom right (hover for the full date), and a line with *Today*, *Yesterday*, the weekday or the date starts each day. Settings > Time format switches between 24-hour and 12-hour.
- **Files**, below the chat, lists the files you sent and received with *Open* and *Show folder* (texts are only in the chat).
- **Clear chat** (above the chat) and **Clear list** (above Files) work independently: clearing one leaves the
  other as it is. They delete on this PC only; the other device keeps its own copy. A phone keeps its history on the
  phone, so what it has already fetched is really deleted here. A note the phone has not fetched yet stays hidden
  until it has (it is deleted at the next Clear). Messages to another PC or phone that were not delivered yet are never
  cleared. Files you received stay in your inbox folder.
- **When the other PC is off:** a message to another PC is kept (shown as "Waiting to send") and a file shows
  "Waiting for NAME to be online". Both are saved and go out by themselves a few seconds after that PC is reachable
  again, also after this program was restarted. Delete the message or press *Cancel* on the file to take it back.
  A phone reads what the PC wrote for it the next time it connects.
- **Zoom:** Ctrl + mouse wheel zooms the whole window, or Ctrl +/- and Ctrl 0, from 60 to 200 %. With the pointer on the chat,
  Ctrl + mouse wheel changes only the size of the messages and the text box (70 to 220 %); *Settings > Chat size > Reset* undoes it.

**Receiving files from another PC:** a card says "NAME wants to send a file" with **Accept** and **Decline**. Nothing is
received until you accept, and offers expire after 15 minutes. You can turn the question off in *Settings > Ask before
receiving a file*. A transfer in progress can be cancelled by either side. For files you send to another PC, *Settings >
Stop a file request that is not accepted after* withdraws the request automatically after that many seconds (0: wait until
the other person answers).

**Pictures** appear in the chat as pictures, not as file cards. A picture that was offered shows a small preview enlarged, so
blurred, and nothing is downloaded until you click it; then it is received and shown sharp. Clicking a received picture opens it.

**Notifications:** while the window is not in front, a new message or file (and a request to send a file or to pair) can show a
popup from the system, play a sound and make the icon ask for attention (the taskbar button flashes, the Dock icon jumps, the tray
icon gets a red dot and blinks). Each of the three has a switch in *Settings > Notifications*.

**Unpair** (right-click a device, or use the button in its conversation) removes the pairing and the conversation here.
A PC you unpair is told and removes its side too; a phone notices the next time it connects.

## Tray icon and hotkey

- The **tray icon**: a click brings the window back, a right click opens the menu with *Open Bridge*, *Send clipboard to phone* and *Quit*. With *Keep running in the tray*
  turned on in Settings, closing the window keeps Bridge running (on a Mac the window goes to the Dock, like the yellow
  button, and a click on the Dock icon brings it back).
- Only one Bridge runs per computer. Starting it again, for example from the taskbar, brings the running window to the front.
- The **hotkey** (default Ctrl+Alt+B) sends the text you copied to your phone from anywhere on the desktop.
  With several phones paired it goes to the phone whose conversation is open, otherwise to the phone that was seen most recently. The same applies to *Send clipboard to phone* in the tray menu.
  *Settings > Send clipboard to phone hotkey* opens a window where you press your own combination: hold as many keys as
  you like, then click *Accept*. *Reset* restores the default. The switch next to it turns the hotkey off.
  On a Mac, combinations that use plain letters need *Accessibility* permission (System Settings > Privacy & Security).

## Settings

This PC's name, **highlight color** (seven ready-made colors or your own), the key that sends a message, the hotkey,
*Visible to other PCs* (switch it off and other PCs stop finding you under Nearby; PCs already paired still work),
tray behavior, the folder for received files, this PC's addresses and its **certificate fingerprint** and the program **version**. The fingerprint is
how phones and other PCs recognize this PC; compare it with the one the phone shows if you want to be sure.

## Where Bridge keeps things

- **Received files:** the folder you chose at first start (default `Downloads/Bridge`).
- **Keys, paired devices, history and settings:** the `pcphone-bridge` folder in your user configuration folder
  (`%APPDATA%` on Windows, `~/Library/Application Support` on macOS, `~/.config` on Linux). Uninstalling does not remove it.
  Delete that folder to start over from scratch.

## Command-line options

Normally not needed. `bridge-app --hidden` starts in the tray (when there is a tray icon). `bridge-app --minimized` starts with the
window minimized to the taskbar or Dock, which also works without a tray (for example on GNOME without the extension, or on
Wayland). Files dropped on Bridge's icon or shortcut (Windows, and Linux launchers that pass files) are accepted as arguments and
ignored: a copy that is already running comes to the front, otherwise Bridge simply starts. `--lan-host 192.168.1.20` sets the address shown to phones if
the detected one is wrong. `--lan-port` (default 8443) and `--peer-port` (default 8444) change the ports, `--inbox <folder>`
and `--data-dir <folder>` change where things are kept, and `--no-discovery` stops Bridge from announcing itself and
looking for other PCs.

## Linux notes

- **Start with the system, minimized:** put a file `~/.config/autostart/bridge.desktop` with `Exec=bridge-app --minimized`
  (or `--hidden` to start in the tray) and the lines of `/usr/share/applications/bridge.desktop` (name, icon, type).

- The tray icon needs a system tray that supports StatusNotifierItem. KDE, XFCE, Cinnamon and most others have one; GNOME needs
  the AppIndicator extension. A left click on the icon shows the window and a right click opens the menu.
  Without a tray, closing the window quits Bridge.
- On a **Wayland** session a window cannot be hidden, so Bridge starts the window through XWayland when *Keep running in the
  tray* is on (it needs a restart after switching it on). Without XWayland, closing the window quits Bridge. On any desktop,
  Bridge ends cleanly when you log out or shut down.
- The global hotkey needs X11 (or XWayland). On a pure Wayland session Settings shows "Hotkey unavailable"; everything else works.
- **Notifications** use `notify-send` (package `libnotify`) and the sound theme (`canberra-gtk-play` or `paplay`). Without them a message still
  makes the window ask for attention; the popup and the sound are skipped.
- The file picker uses the desktop portal (`xdg-desktop-portal`), which current desktops include.

## If something does not work

- **The phone cannot connect:** phone and PC must be on the same Wi-Fi or network, and the firewall must allow Bridge on
  private networks (ports above). "Guest" Wi-Fi or "client isolation" settings in the router block devices from seeing each other.
- **Another PC does not show under Nearby:** some routers block the multicast messages used for discovery. Use
  *Pair by address...* instead.
- **The address in the QR code is wrong** (several network cards or a VPN): start Bridge with `--lan-host` and the right address.
- **A file offer disappeared:** unanswered offers are dropped after 15 minutes or when either PC restarts. Send it again.

## Limits

- Whole folders cannot be sent; zip them first.
- PC-to-PC works on one network and with IPv4.
- Deleting a message while the other PC is offline does not reach that PC later.
