package dev.bridge.phone

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

fun shortFingerprint(fingerprint: String): String = fingerprint.take(16).chunked(4).joinToString(" ")

@Composable
fun SettingsScreen(onPair: () -> Unit) {
    val p = LocalPalette.current
    val pcs by Bridge.pcs.collectAsState()
    val active by Bridge.active.collectAsState()
    var name by remember { mutableStateOf(Bridge.phoneName) }
    var editing by remember { mutableStateOf(false) }
    var leaving by remember { mutableStateOf<Pc?>(null) }
    var leavingPhone by remember { mutableStateOf<dev.bridge.phone.p2p.Peer?>(null) }
    val phones by Phones.peers.collectAsState()
    val activePhone by Phones.active.collectAsState()
    val visible by Phones.visibleToPhones.collectAsState()
    val background by Phones.receiveInBackground.collectAsState()
    val address by Phones.address.collectAsState()
    val mode by Bridge.themeMode.collectAsState()
    val time24 by Bridge.time24.collectAsState()
    val context = LocalContext.current
    val version = remember { runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "" }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).statusBarsPadding().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Txt("Settings", Modifier.padding(start = 4.dp, top = 8.dp), size = 32.sp, weight = FontWeight.Bold)

        SectionLabel("THIS PHONE")
        Panel {
            Row(
                Modifier.fillMaxWidth().clickable(role = Role.Button) { editing = true }.padding(horizontal = 16.dp, vertical = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Txt("Name")
                    Txt("Shown to your other devices", color = p.label2, size = 13.sp)
                }
                Txt(name, color = p.label2, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Sep()
            SwitchRow("Visible to other phones", "Lets phones on this Wi-Fi find this one when you pair", visible) { Phones.setVisible(it) }
            Sep()
            SwitchRow("Receive while app is closed", "Keeps a small notification so Bridge can receive in the background", background) { Phones.setBackground(it) }
            Sep()
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Txt("Address of this phone")
                    Txt("For Pair by address", color = p.label2, size = 13.sp)
                }
                Txt(address ?: "Not on Wi-Fi", color = p.label2)
            }
        }
        Footnote("This is the name other phones show for this phone. A PC shows the name this phone had when it was paired; rename it in Bridge on the PC.")

        SectionLabel("APPEARANCE")
        Panel {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Txt("Dark mode", Modifier.weight(1f))
                listOf(ThemeMode.System to "System", ThemeMode.Light to "Off", ThemeMode.Dark to "On").forEach { (value, label) ->
                    val selected = value == mode
                    Box(
                        Modifier.height(36.dp).clip(RoundedCornerShape(18.dp))
                            .background(if (selected) p.tint else p.field)
                            .clickable(role = Role.RadioButton) { Bridge.setThemeMode(value) }
                            .padding(horizontal = 12.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Txt(label, color = if (selected) Color.White else p.label, size = 14.sp, weight = if (selected) FontWeight.SemiBold else null)
                    }
                }
            }
        }
        Footnote("System follows the phone's own dark mode setting.")
        Panel {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Txt("Time format", Modifier.weight(1f))
                listOf(true to "24-hour", false to "12-hour").forEach { (value, label) ->
                    val selected = value == time24
                    Box(
                        Modifier.height(36.dp).clip(RoundedCornerShape(18.dp))
                            .background(if (selected) p.tint else p.field)
                            .clickable(role = Role.RadioButton) { Bridge.setTime24(value) }
                            .padding(horizontal = 12.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Txt(label, color = if (selected) Color.White else p.label, size = 14.sp, weight = if (selected) FontWeight.SemiBold else null)
                    }
                }
            }
        }
        Footnote("How the time under each message is written, for example 21:14 or 9:14 PM.")

        SectionLabel("PAIRED DEVICES")
        Panel {
            if (pcs.isEmpty() && phones.isEmpty()) Txt("Nothing is paired.", Modifier.padding(16.dp), color = p.label2)
            pcs.forEachIndexed { i, pc ->
                if (i > 0) Sep()
                Row(
                    Modifier.fillMaxWidth().clickable(role = Role.RadioButton) { Bridge.select(pc) }.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Box(Modifier.size(36.dp).clip(RoundedCornerShape(10.dp)).background(p.field), contentAlignment = Alignment.Center) {
                        Icn(R.drawable.ic_monitor, p.label, 20.dp)
                    }
                    Column(Modifier.weight(1f)) {
                        Txt(pc.name, weight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Txt("PC · ${pc.host}:${pc.port}", color = p.label2, size = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Txt("Certificate ${shortFingerprint(pc.fingerprint)}", color = p.label2, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    if (pc.id == active?.id && activePhone == null) Icn(R.drawable.ic_check, p.tintText, 22.dp)
                    TextAction("Remove", { leaving = pc }, color = p.red)
                }
            }
            phones.forEachIndexed { i, ph ->
                if (i > 0 || pcs.isNotEmpty()) Sep()
                Row(
                    Modifier.fillMaxWidth().clickable(role = Role.RadioButton) { Phones.select(ph) }.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Box(Modifier.size(36.dp).clip(RoundedCornerShape(10.dp)).background(p.field), contentAlignment = Alignment.Center) {
                        Icn(R.drawable.ic_phone, p.label, 20.dp)
                    }
                    Column(Modifier.weight(1f)) {
                        Txt(ph.name, weight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Txt("Phone", color = p.label2, size = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Txt("Certificate ${shortFingerprint(ph.fp)}", color = p.label2, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    if (activePhone?.fp == ph.fp) Icn(R.drawable.ic_check, p.tintText, 22.dp)
                    TextAction("Remove", { leavingPhone = ph }, color = p.red)
                }
            }
        }
        Pill("Pair a device", onPair, Modifier.fillMaxWidth(), height = 48.dp)
        Footnote("Removing a device also deletes your conversation with it on this phone. The certificate fingerprint is how this phone recognizes a device; compare it with the one shown on the other device if you ever want to be sure.")

        SectionLabel("ABOUT")
        Panel {
            Txt("Files you receive are saved in Downloads/Bridge.", Modifier.padding(horizontal = 16.dp, vertical = 14.dp))
            Sep()
            Txt("Everything stays on your network. Connections between your devices are encrypted. This phone only talks to a device whose certificate it was paired with.", Modifier.padding(horizontal = 16.dp, vertical = 14.dp))
            Sep()
            Txt("Version $version", Modifier.padding(horizontal = 16.dp, vertical = 14.dp), color = p.label2)
        }
        Spacer(Modifier.height(16.dp))
    }

    if (editing) {
        var draft by remember { mutableStateOf(name) }
        AlertDialog(
            onDismissRequest = { editing = false },
            containerColor = p.card,
            title = { Text("Phone name", color = p.label) },
            text = { TextField(value = draft, onValueChange = { draft = it.take(40) }, singleLine = true) },
            confirmButton = {
                TextAction("Save", {
                    Bridge.phoneName = draft
                    name = Bridge.phoneName
                    Phones.nameChanged()
                    editing = false
                })
            },
            dismissButton = { TextAction("Cancel", { editing = false }) },
        )
    }

    leavingPhone?.let { ph ->
        AlertDialog(
            onDismissRequest = { leavingPhone = null },
            containerColor = p.card,
            title = { Text("Remove “${ph.name}”?", color = p.label) },
            text = { Text("This phone forgets ${ph.name} and deletes your conversation with it here. ${ph.name} is told, and forgets this phone too. To use them together again you pair them with a new code.", color = p.label2) },
            confirmButton = {
                TextAction("Remove", {
                    Phones.unpair(ph)
                    leavingPhone = null
                }, color = p.red)
            },
            dismissButton = { TextAction("Cancel", { leavingPhone = null }) },
        )
    }

    leaving?.let { pc ->
        AlertDialog(
            onDismissRequest = { leaving = null },
            containerColor = p.card,
            title = { Text("Remove “${pc.name}”?", color = p.label) },
            text = { Text("This phone forgets the PC, and the PC forgets this phone. To use them together again you pair them with a new code.", color = p.label2) },
            confirmButton = {
                TextAction("Remove", {
                    Bridge.unpair(pc)
                    leaving = null
                }, color = p.red)
            },
            dismissButton = { TextAction("Cancel", { leaving = null }) },
        )
    }
}

@Composable
private fun SwitchRow(title: String, text: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    val p = LocalPalette.current
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 12.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Column(Modifier.weight(1f)) {
            Txt(title)
            Txt(text, color = p.label2, size = 13.sp)
        }
        Switch(checked = checked, onCheckedChange = onChange)
    }
}
