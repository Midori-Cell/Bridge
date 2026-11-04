package dev.bridge.phone

import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.os.Bundle
import android.util.Size
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.IntentCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** What another app shared with Bridge: files, or some text. */
data class Shared(val uris: List<Uri>, val text: String?)

/** Mockup F: the sheet that opens from Android's Share menu and asks where to send. */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val uris: List<Uri> = when (intent.action) {
            Intent.ACTION_SEND -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
            Intent.ACTION_SEND_MULTIPLE -> IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            else -> emptyList()
        }
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.takeIf { it.isNotBlank() }
        if (uris.isEmpty() && text == null) {
            finish()
            return
        }
        val shared = Shared(uris, if (uris.isEmpty()) text else null)
        setContent {
            BridgeTheme {
                ShareSheet(
                    shared = shared,
                    onSend = { pc ->
                        if (shared.text != null) {
                            Bridge.sendTextInBackground(shared.text, pc)
                        } else {
                            TransferService.sendFiles(this, shared.uris, pc)
                        }
                        finish()
                    },
                    onOpenApp = {
                        startActivity(Intent(this, MainActivity::class.java))
                        finish()
                    },
                    onClose = { finish() },
                )
            }
        }
    }
}

@Composable
private fun ShareSheet(shared: Shared, onSend: (Pc) -> Unit, onOpenApp: () -> Unit, onClose: () -> Unit) {
    val p = LocalPalette.current
    val context = androidx.compose.ui.platform.LocalContext.current
    val pcs by Bridge.pcs.collectAsState()
    val active by Bridge.active.collectAsState()
    var chosen by remember { mutableStateOf<String?>(null) }
    val target = pcs.firstOrNull { it.id == (chosen ?: active?.id) } ?: pcs.firstOrNull()

    // what is being shared, in words
    val infos = remember(shared) { shared.uris.map { Bridge.describe(it) } }
    val types = remember(shared) { shared.uris.map { context.contentResolver.getType(it).orEmpty() } }
    val noun = when {
        shared.text != null -> "text"
        types.isNotEmpty() && types.all { it.startsWith("image/") } -> "photo"
        types.isNotEmpty() && types.all { it.startsWith("video/") } -> "video"
        else -> "file"
    }
    val count = shared.uris.size
    val title = if (shared.text != null) "Text" else if (count == 1) "1 $noun" else "$count ${noun}s"
    val detail = if (shared.text != null) {
        shared.text.take(80)
    } else {
        val names = infos.take(2).joinToString(", ") { it.first } + if (count > 2) ", ..." else ""
        "$names · ${sizeText(infos.sumOf { it.second })}"
    }
    val sendLabel = if (shared.text != null) "Send text" else "Send " + (if (count == 1) "1 $noun" else "$count ${noun}s")

    val thumbnail by produceState<Bitmap?>(null, shared) {
        val first = shared.uris.firstOrNull() ?: return@produceState
        value = withContext(Dispatchers.IO) {
            try {
                context.contentResolver.loadThumbnail(first, Size(160, 160), null)
            } catch (e: Exception) {
                null
            }
        }
    }

    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.5f)).clickable(onClick = onClose))
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                .clip(RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)).background(p.bg)
                .navigationBarsPadding().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(Modifier.align(Alignment.CenterHorizontally).size(width = 36.dp, height = 5.dp).clip(RoundedCornerShape(3.dp)).background(p.label2.copy(alpha = 0.6f)))

            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(p.card).padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Box(Modifier.size(64.dp).clip(RoundedCornerShape(10.dp)).background(p.field), contentAlignment = Alignment.Center) {
                    val bitmap = thumbnail
                    if (bitmap != null) {
                        Image(bitmap.asImageBitmap(), contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                    } else {
                        Icn(R.drawable.ic_file, p.label2, 28.dp)
                    }
                }
                Column(Modifier.weight(1f)) {
                    Txt(title, weight = FontWeight.SemiBold)
                    Txt(detail, color = p.label2, size = 13.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }

            if (pcs.isEmpty()) {
                Panel {
                    Txt("No PC is paired yet. Open Bridge and pair one first.", Modifier.padding(16.dp), color = p.label2)
                }
                Pill("Open Bridge", onOpenApp, Modifier.fillMaxWidth(), primary = true, height = 48.dp)
            } else {
                SectionLabel("SEND TO")
                Panel {
                    pcs.forEachIndexed { i, pc ->
                        if (i > 0) Sep()
                        Row(
                            Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.RadioButton) { chosen = pc.id }.padding(horizontal = 16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Box(Modifier.size(36.dp).clip(RoundedCornerShape(10.dp)).background(p.field), contentAlignment = Alignment.Center) {
                                Icn(R.drawable.ic_monitor, p.label, 20.dp)
                            }
                            Txt(pc.name, Modifier.weight(1f), weight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (pc.id == target?.id) Icn(R.drawable.ic_check, p.tintText, 22.dp)
                        }
                    }
                }
                Footnote("Files go straight to your PC over your home network. Nothing is uploaded to the internet.")
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Pill("Cancel", onClose, Modifier.weight(1f), height = 48.dp)
                    Pill(sendLabel, { target?.let(onSend) }, Modifier.weight(1.4f), primary = true, height = 48.dp)
                }
            }
        }
    }
}
