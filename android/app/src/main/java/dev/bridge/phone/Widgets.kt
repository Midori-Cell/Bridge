package dev.bridge.phone

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.widget.Toast
import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.text.DateFormat
import java.util.Date

@Composable
fun Txt(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = LocalPalette.current.label,
    size: TextUnit = 16.sp,
    weight: FontWeight? = null,
    maxLines: Int = Int.MAX_VALUE,
    overflow: TextOverflow = TextOverflow.Clip,
    align: TextAlign? = null,
) {
    Text(text = text, modifier = modifier, color = color, fontSize = size, fontWeight = weight, maxLines = maxLines, overflow = overflow, textAlign = align)
}

@Composable
fun Icn(@DrawableRes id: Int, tint: Color, size: Dp = 24.dp, modifier: Modifier = Modifier) {
    Icon(painter = painterResource(id), contentDescription = null, tint = tint, modifier = modifier.size(size))
}

/** A rounded button like the ones in the mockups. */
@Composable
fun Pill(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    primary: Boolean = false,
    danger: Boolean = false,
    enabled: Boolean = true,
    height: Dp = 44.dp,
    padding: Dp = 16.dp,
) {
    val p = LocalPalette.current
    val background = if (primary) p.tint else p.field
    val foreground = when {
        primary -> Color.White
        danger -> p.red
        else -> p.label
    }
    Box(
        modifier
            .height(height)
            .alpha(if (enabled) 1f else 0.4f)
            .clip(RoundedCornerShape(height / 2))
            .background(background)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = padding),
        contentAlignment = Alignment.Center,
    ) {
        Txt(label, color = foreground, size = 15.sp, weight = if (primary) FontWeight.SemiBold else null)
    }
}

/** A text-only action ("Change", "Open", "Copy") with a touch target of at least 44 dp. */
@Composable
fun TextAction(label: String, onClick: () -> Unit, modifier: Modifier = Modifier, color: Color = LocalPalette.current.tintText) {
    Box(
        modifier.heightIn(min = 44.dp).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Txt(label, color = color, size = 15.sp)
    }
}

/** A white (or dark grey) rounded block that holds rows. */
@Composable
fun Panel(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(LocalPalette.current.card), content = content)
}

@Composable
fun Sep() {
    Box(Modifier.fillMaxWidth().padding(start = 16.dp).height(1.dp).background(LocalPalette.current.sep))
}

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Txt(text, modifier.padding(start = 6.dp, top = 4.dp), color = LocalPalette.current.label2, size = 13.sp, weight = FontWeight.SemiBold)
}

@Composable
fun Footnote(text: String, modifier: Modifier = Modifier) {
    Txt(text, modifier.padding(horizontal = 6.dp), color = LocalPalette.current.label2, size = 13.sp)
}

/** The chat window's outline: left, top and right, with rounded top corners and an open bottom. */
fun Modifier.topBorder(color: Color, radius: Dp, width: Dp = 1.5.dp): Modifier = drawWithContent {
    drawContent() // the line is drawn last, so messages scroll under it, not over it
    val w = size.width
    val h = size.height
    val r = radius.toPx()
    val sw = width.toPx()
    val half = sw / 2
    val d = 2 * (r - half)
    val path = Path().apply {
        moveTo(half, h)
        lineTo(half, r)
        arcTo(Rect(half, half, half + d, half + d), 180f, 90f, false)
        lineTo(w - r, half)
        arcTo(Rect(w - half - d, half, w - half, half + d), 270f, 90f, false)
        lineTo(w - half, h)
    }
    drawPath(path, color, style = Stroke(sw))
}

// ---------------------------------------------------------------------- small helpers

fun sizeText(n: Long): String {
    if (n < 1024) return "$n B"
    val units = listOf("KB", "MB", "GB", "TB")
    var v = n / 1024.0
    var i = 0
    while (v >= 1024 && i < units.size - 1) {
        v /= 1024
        i++
    }
    return (if (v >= 100) "%.0f".format(v) else "%.1f".format(v)) + " " + units[i]
}

fun whenText(ts: Double): String {
    val secs = (System.currentTimeMillis() / 1000.0 - ts).toLong()
    return when {
        secs < 45 -> "just now"
        secs < 3600 -> "${maxOf(1L, secs / 60)} min ago"
        secs < 86400 -> "${secs / 3600} h ago"
        else -> DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date((ts * 1000).toLong()))
    }
}

fun copyText(context: Context, text: String) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText("Bridge", text))
    if (Build.VERSION.SDK_INT < 33) Toast.makeText(context, "Copied", Toast.LENGTH_SHORT).show() // newer Android shows its own message
}

fun clipboardText(context: Context): String? {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    val clip = clipboard.primaryClip ?: return null
    if (clip.itemCount == 0) return null
    return clip.getItemAt(0).coerceToText(context)?.toString()?.takeIf { it.isNotEmpty() }
}

fun toast(context: Context, text: String) {
    Toast.makeText(context, text, Toast.LENGTH_LONG).show()
}

/** How a file note looks and what tapping it does. `action` is null while nothing can be done. */
data class FileState(val action: String?, val status: String)

fun fileState(note: Note, transfers: List<Transfer>, saved: Map<String, String>): FileState {
    val file = note.file ?: return FileState(null, "")
    if (file.dir != "to_phone") return FileState(null, "${sizeText(file.size)} · sent")
    val t = transfers.firstOrNull { it.key == file.id }
    return when {
        t != null && t.running -> FileState(null, if (t.state == TState.Waiting) "Waiting..." else "${percent(t)}%")
        file.id in saved -> FileState("Open", "${sizeText(file.size)} · saved in Downloads/Bridge")
        t != null && t.state == TState.Failed -> FileState("Retry", "Failed: ${t.error ?: "unknown error"}")
        else -> FileState("Save", "${sizeText(file.size)} · not saved yet")
    }
}

fun percent(t: Transfer): Int = if (t.size > 0) (t.done * 100 / t.size).toInt().coerceIn(0, 100) else 0

/** Open a saved file, or fetch it from the PC. */
fun fileTap(context: Context, note: Note, pc: Pc?, saved: Map<String, String>) {
    val file = note.file ?: return
    if (file.dir != "to_phone") return
    if (file.id in saved) Bridge.open(context, file) else if (pc != null) Bridge.download(file, pc)
}

// ---------------------------------------------------------------------- web addresses in messages

class LinkSpan(val start: Int, val end: Int, val url: String)

/** Find `http://`, `https://` and `www.` addresses. Punctuation that ends a sentence is not part of the address. */
fun findLinks(text: String): List<LinkSpan> {
    val out = mutableListOf<LinkSpan>()
    var i = 0
    while (i < text.length) {
        val boundary = i == 0 || text[i - 1].isWhitespace() || text[i - 1] in "([{<\"'"
        if (boundary) {
            val scheme = when {
                text.startsWith("https://", i, ignoreCase = true) -> 8
                text.startsWith("http://", i, ignoreCase = true) -> 7
                text.startsWith("www.", i, ignoreCase = true) -> 0
                else -> -1
            }
            if (scheme >= 0) {
                var j = i
                while (j < text.length && !text[j].isWhitespace()) j++
                while (j > i) {
                    val c = text[j - 1]
                    val token = text.substring(i, j)
                    val trim = c in ".,;:!?'\"*>" ||
                        (c == ')' && token.count { it == '(' } < token.count { it == ')' }) ||
                        (c == ']' && token.count { it == '[' } < token.count { it == ']' }) ||
                        (c == '}' && token.count { it == '{' } < token.count { it == '}' })
                    if (!trim) break
                    j--
                }
                val prefix = if (scheme == 0) 4 else scheme
                if (j > i + prefix) {
                    val shown = text.substring(i, j)
                    out.add(LinkSpan(i, j, if (scheme == 0) "https://$shown" else shown))
                    i = j
                    continue
                }
            }
        }
        i++
    }
    return out
}

/** The message text with its web addresses underlined; tapping one opens it in the browser. */
fun linkify(text: String, linkColor: Color): AnnotatedString {
    val links = findLinks(text)
    if (links.isEmpty()) return AnnotatedString(text)
    val style = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
    return buildAnnotatedString {
        var at = 0
        for (l in links) {
            append(text.substring(at, l.start))
            withLink(LinkAnnotation.Url(l.url, style)) { append(text.substring(l.start, l.end)) }
            at = l.end
        }
        append(text.substring(at))
    }
}
