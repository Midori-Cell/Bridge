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
                Txt("Name", Modifier.weight(1f))
                Txt(name, color = p.label2, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        Footnote("This is the name a PC shows for this phone. A new name is used the next time you pair; to rename a phone that is already paired, do it in Bridge on the PC.")

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

        SectionLabel("PAIRED PCS")
        Panel {
            if (pcs.isEmpty()) Txt("No PC is paired.", Modifier.padding(16.dp), color = p.label2)
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
                        Txt("${pc.host}:${pc.port}", color = p.label2, size = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Txt("Certificate ${shortFingerprint(pc.fingerprint)}", color = p.label2, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    if (pc.id == active?.id) Icn(R.drawable.ic_check, p.tintText, 22.dp)
                    TextAction("Unpair", { leaving = pc }, color = p.red)
                }
            }
        }
        Pill("Pair another PC", onPair, Modifier.fillMaxWidth(), height = 48.dp)
        Footnote("The certificate fingerprint is how this phone recognizes the PC. Compare it with the certificate fingerprint in Settings in Bridge on the PC if you ever want to be sure.")

        SectionLabel("ABOUT")
        Panel {
            Txt("Files from a PC are saved in Downloads/Bridge.", Modifier.padding(horizontal = 16.dp, vertical = 14.dp))
            Sep()
            Txt("Everything stays on your network. Connections are encrypted. This phone only talks to a PC whose certificate it was paired with.", Modifier.padding(horizontal = 16.dp, vertical = 14.dp))
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
                    editing = false
                })
            },
            dismissButton = { TextAction("Cancel", { editing = false }) },
        )
    }

    leaving?.let { pc ->
        AlertDialog(
            onDismissRequest = { leaving = null },
            containerColor = p.card,
            title = { Text("Unpair “${pc.name}”?", color = p.label) },
            text = { Text("This phone forgets the PC, and the PC forgets this phone. To use them together again you pair them with a new code.", color = p.label2) },
            confirmButton = {
                TextAction("Unpair", {
                    Bridge.unpair(pc)
                    leaving = null
                }, color = p.red)
            },
            dismissButton = { TextAction("Cancel", { leaving = null }) },
        )
    }
}
