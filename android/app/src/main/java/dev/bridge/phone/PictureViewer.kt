package dev.bridge.phone

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import androidx.compose.ui.platform.LocalView
import androidx.compose.runtime.SideEffect
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlin.math.max
import kotlin.math.min

/** Most a picture can be enlarged, relative to fitting the screen. */
private const val MAX_SCALE = 6f

/**
 * Pictures of the chat shown large, inside the app: swipe for the other pictures, pinch (or double-tap) to zoom,
 * drag to move a zoomed picture, tap to hide or show the bar, Back or × to close.
 */
@Composable
fun PictureViewer(pics: List<FileRef>, start: Int, onClose: () -> Unit) {
    if (pics.isEmpty()) return
    val context = LocalContext.current
    val saved by Bridge.saved.collectAsState()
    var bars by remember { mutableStateOf(true) }
    val pager = rememberPagerState(initialPage = start.coerceIn(0, pics.size - 1)) { pics.size }
    // Opening and closing: the view fades and grows in, and fades and shrinks out again before it is gone.
    // (Android's own dialog animation cannot run on closing, because the content is removed at once.)
    val appear = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    var closing by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { appear.animateTo(1f, tween(200, easing = FastOutSlowInEasing)) }
    val close: () -> Unit = {
        if (!closing) {
            closing = true
            scope.launch {
                appear.animateTo(0f, tween(180, easing = FastOutSlowInEasing))
                onClose()
            }
        }
    }
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        // no window animation and no dimming from Android: the fade above does both ways, the same way
        val window = (LocalView.current.parent as? DialogWindowProvider)?.window
        SideEffect {
            window?.setWindowAnimations(0)
            window?.setDimAmount(0f)
        }
        Box(
            Modifier.fillMaxSize()
                .graphicsLayer {
                    alpha = appear.value
                    val s = 0.92f + 0.08f * appear.value
                    scaleX = s
                    scaleY = s
                }
                .background(Color.Black),
        ) {
            HorizontalPager(state = pager, modifier = Modifier.fillMaxSize(), key = { pics[it].id }) { page ->
                ZoomablePicture(pics[page], active = pager.currentPage == page, onTap = { bars = !bars })
            }
            if (bars) {
                val f = pics[pager.currentPage.coerceIn(0, pics.size - 1)]
                Row(
                    Modifier.fillMaxWidth()
                        .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.75f), Color.Transparent)))
                        .statusBarsPadding()
                        .padding(horizontal = 8.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Box(Modifier.size(44.dp).clip(CircleShape).clickable { close() }, contentAlignment = Alignment.Center) {
                        Icn(R.drawable.ic_close, Color.White, 22.dp)
                    }
                    Column(Modifier.weight(1f)) {
                        Txt(f.name, color = Color.White, size = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        val where = if (pics.size > 1) "${pager.currentPage + 1} of ${pics.size} · " else ""
                        Txt(where + sizeText(f.size), color = Color.White.copy(alpha = 0.7f), size = 12.sp, maxLines = 1)
                    }
                    if (saved[f.id] != null) {
                        // the system's gallery or another app, for editing or sharing
                        Box(Modifier.clip(CircleShape).clickable { Bridge.open(context, f) }.padding(horizontal = 12.dp, vertical = 10.dp)) {
                            Txt("Open in…", color = Color.White, size = 14.sp)
                        }
                    }
                }
                if (pics.size > 1) {
                    Txt(
                        "Swipe for the other pictures · pinch to zoom",
                        Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = 14.dp),
                        color = Color.White.copy(alpha = 0.55f),
                        size = 12.sp,
                    )
                }
            }
        }
    }
}

@Composable
private fun ZoomablePicture(file: FileRef, active: Boolean, onTap: () -> Unit) {
    val context = LocalContext.current
    val saved by Bridge.saved.collectAsState()
    val savedUri = saved[file.id]
    val small: Bitmap? = remember(file.id) { Thumbs.sharpCached(file.id, savedUri) ?: file.thumb?.let { Thumbs.small(it) } }
    val large by produceState<Bitmap?>(null, file.id, savedUri) {
        value = withContext(Dispatchers.IO) { Thumbs.large(context, file.id, savedUri) }
    }
    var waitedLong by remember(file.id) { mutableStateOf(false) }
    LaunchedEffect(file.id, savedUri) {
        kotlinx.coroutines.delay(4000)
        waitedLong = true // still no picture after a while: it is gone
    }
    // the gesture code below reads and writes these directly, so it always sees the newest values
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    LaunchedEffect(active) {
        if (!active) {
            scale = 1f
            offset = Offset.Zero
        }
    }
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val boxW = constraints.maxWidth.toFloat()
        val boxH = constraints.maxHeight.toFloat()
        val shown = large ?: small
        if (shown == null) {
            Txt(if (waitedLong) "This picture is no longer on the phone." else "Loading…", color = Color.White.copy(alpha = 0.7f), size = 14.sp)
        } else {
            // the picture fitted into the screen, in pixels
            val fit = min(boxW / shown.width, boxH / shown.height)
            val w = shown.width * fit
            val h = shown.height * fit
            val clamp = { o: Offset, s: Float ->
                val mx = max(0f, (w * s - boxW) / 2f)
                val my = max(0f, (h * s - boxH) / 2f)
                Offset(o.x.coerceIn(-mx, mx), o.y.coerceIn(-my, my))
            }
            // keep the point under the fingers where it is
            val zoomTo = { target: Float, at: Offset ->
                val c = at - Offset(boxW / 2f, boxH / 2f)
                val next = target.coerceIn(1f, MAX_SCALE)
                offset = if (next <= 1f) Offset.Zero else clamp(c - (c - offset) * (next / scale), next)
                scale = next
            }
            val density = LocalDensity.current
            Image(
                shown.asImageBitmap(),
                contentDescription = file.name,
                contentScale = ContentScale.Fit,
                modifier = with(density) { Modifier.size(w.toDp(), h.toDp()) }
                    .graphicsLayer {
                        scaleX = scale
                        scaleY = scale
                        translationX = offset.x
                        translationY = offset.y
                    },
            )
            // the gestures cover the whole page, so a zoomed picture can be moved from anywhere
            Box(
                Modifier.fillMaxSize()
                    .pointerInput(file.id, w, h) {
                        awaitEachGesture {
                            awaitFirstDown(requireUnconsumed = false)
                            do {
                                val event = awaitPointerEvent()
                                val fingers = event.changes.count { it.pressed }
                                // one finger on a picture that is not zoomed is a swipe to the next picture: left alone
                                if (fingers > 1 || scale > 1f) {
                                    val zoom = event.calculateZoom()
                                    val pan = event.calculatePan()
                                    val centroid = event.calculateCentroid(useCurrent = false)
                                    if (zoom != 1f && centroid != Offset.Unspecified) zoomTo(scale * zoom, centroid)
                                    if (scale > 1f) offset = clamp(offset + pan, scale)
                                    event.changes.forEach { if (it.positionChanged()) it.consume() }
                                }
                            } while (event.changes.any { it.pressed })
                        }
                    }
                    .pointerInput(file.id, w, h) {
                        detectTapGestures(
                            onTap = { onTap() },
                            onDoubleTap = { at -> zoomTo(if (scale > 1.05f) 1f else 2.5f, at) },
                        )
                    },
            )
        }
    }
}
