package dev.bridge.phone

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * A picture in the chat. Until it was downloaded only its tiny preview is there, enlarged (so blurred) with a note on top;
 * tapping the bubble downloads it. Once the phone has it, the picture is shown sharp and tapping opens it.
 */
@Composable
fun PictureContent(file: FileRef, state: FileState, quiet: Color) {
    val context = LocalContext.current
    val saved by Bridge.saved.collectAsState()
    val savedUri = saved[file.id]
    val small: Bitmap? = remember(file.thumb) { file.thumb?.let { Thumbs.small(it) } }
    val sharp by produceState<Bitmap?>(null, file.id, savedUri) {
        value = withContext(Dispatchers.IO) { Thumbs.sharp(context, file.id, savedUri) }
    }
    val shown = sharp ?: small
    val ratio = if (shown != null && shown.height > 0) shown.width.toFloat() / shown.height else 1.3f
    val p = LocalPalette.current
    Column {
        Box(
            Modifier.width(220.dp).aspectRatio(ratio.coerceIn(0.6f, 1.8f)).clip(RoundedCornerShape(10.dp)).background(p.field),
        ) {
            if (shown != null) {
                // a small image drawn large with smoothing is what makes the preview blurred
                Image(shown.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop, filterQuality = FilterQuality.Low)
            }
            if (sharp == null && file.dir == "to_phone") {
                val label = when {
                    state.running -> state.status
                    state.saved -> "Tap to open"
                    state.declined -> "Declined"
                    state.failed -> "Failed · tap to retry"
                    else -> "Tap to download · ${sizeText(file.size)}"
                }
                Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.22f)), contentAlignment = Alignment.Center) {
                    Box(Modifier.clip(RoundedCornerShape(14.dp)).background(Color.Black.copy(alpha = 0.55f)).padding(horizontal = 12.dp, vertical = 6.dp)) {
                        Txt(label, color = Color.White, size = 13.sp, maxLines = 2)
                    }
                }
            }
        }
        Txt(file.name, Modifier.padding(top = 4.dp).width(220.dp), color = quiet, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
