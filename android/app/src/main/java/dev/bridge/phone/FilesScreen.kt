package dev.bridge.phone

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** The Files tab: what is on its way, and every file that went back and forth. */
@Composable
fun FilesScreen() {
    val p = LocalPalette.current
    val context = LocalContext.current
    val pc by Bridge.active.collectAsState()
    val notes by Bridge.notes.collectAsState()
    val transfers by Bridge.transfers.collectAsState()
    val saved by Bridge.saved.collectAsState()
    val files = notes.filter { it.isFile }.asReversed()

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).statusBarsPadding().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Txt("Files", Modifier.padding(start = 4.dp, top = 8.dp), size = 32.sp, weight = FontWeight.Bold)

        if (transfers.isNotEmpty()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SectionLabel("TRANSFERS", Modifier.weight(1f))
                TextAction("Clear", { Bridge.clearFinished() })
            }
            Panel {
                transfers.asReversed().forEachIndexed { i, t ->
                    if (i > 0) Sep()
                    TransferRow(t)
                }
            }
        }

        SectionLabel("FILES")
        Panel {
            if (files.isEmpty()) {
                Txt("No files yet. Use Files on the Notes tab, or share files from another app to Bridge.", Modifier.padding(16.dp), color = p.label2, size = 14.sp)
            }
            files.forEachIndexed { i, note ->
                if (i > 0) Sep()
                val file = note.file ?: return@forEachIndexed
                val state = fileState(note, transfers, saved)
                Row(
                    Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 6.dp, bottom = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Box(Modifier.size(40.dp).clip(RoundedCornerShape(8.dp)).background(p.field), contentAlignment = Alignment.Center) {
                        Icn(R.drawable.ic_file, p.label, 20.dp)
                    }
                    Column(Modifier.weight(1f).padding(vertical = 6.dp)) {
                        Txt(file.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Txt("${if (note.fromPc) pc?.name ?: "PC" else "You"} · ${whenText(note.ts)}", color = p.label2, size = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Txt(state.status, color = p.label2, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    state.action?.let { TextAction(it, { fileTap(context, note, pc, saved) }) }
                }
            }
        }
        Footnote("Received files are saved in Downloads/Bridge.")
        Spacer(Modifier.height(16.dp))
    }
}

@Composable
private fun TransferRow(t: Transfer) {
    val p = LocalPalette.current
    val status = when (t.state) {
        TState.Waiting -> if (t.upload) "Waiting for the PC to accept..." else "Waiting..."
        TState.Queued -> "Waiting for the PC to be online. It is sent when the PC is back."
        TState.Active -> "${percent(t)}% of ${sizeText(t.size)}"
        TState.Done -> if (t.upload) "Sent · ${sizeText(t.size)}" else "Saved · ${sizeText(t.size)}"
        TState.Failed -> t.error ?: "Failed"
    }
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Txt(t.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (t.running) {
                Box(Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(p.field)) {
                    Box(Modifier.fillMaxWidth(percent(t) / 100f).fillMaxHeight().background(p.tint))
                }
            }
            Txt(status, color = if (t.state == TState.Failed) p.red else p.label2, size = 13.sp)
        }
        if (t.running) {
            TextAction("Cancel", { Bridge.cancel(t.key) })
        } else {
            TextAction("Remove", { Bridge.dismiss(t.key) })
        }
    }
}
