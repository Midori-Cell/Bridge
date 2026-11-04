package dev.bridge.phone

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.OptIn
import androidx.camera.core.CameraSelector
import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import com.google.mlkit.vision.barcode.BarcodeScanner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import kotlinx.coroutines.launch
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import java.util.concurrent.Executors

/** "host" or "host:port" (8443 unless given) as typed by the user. */
private fun parseAddress(text: String): Pair<String, Int>? {
    val typed = text.trim().removePrefix("https://").trimEnd('/')
    val url = ("https://" + typed).toHttpUrlOrNull() ?: return null
    val typedPort = typed.substringAfterLast(']').contains(':')
    return url.host to (if (typedPort) url.port else 8443)
}

/** Mockup G: scan the QR code on the PC, or type the address and code. */
@Composable
fun PairScreen(canLeave: Boolean, onDone: () -> Unit, onLeave: () -> Unit) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var manual by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var lastTried by remember { mutableStateOf<String?>(null) }
    var address by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var verify by remember { mutableStateOf<Pc?>(null) }
    var granted by remember { mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }

    fun attempt(host: String, port: Int, code: String, fingerprint: String?) {
        busy = true
        error = null
        scope.launch {
            when (val result = Bridge.pair(host, port, code, fingerprint)) {
                is PairResult.Ok -> {
                    busy = false
                    if (result.unverified) verify = result.pc else onDone()
                }
                is PairResult.Failed -> {
                    busy = false
                    error = result.message
                }
            }
        }
    }

    fun scanned(raw: String) {
        if (busy || raw == lastTried) return
        lastTried = raw
        val link = parsePairLink(raw)
        if (link == null) {
            error = "That is not a Bridge code. On the PC choose Pair a device, then Pair a phone, and scan the QR code shown there."
        } else {
            attempt(link.host, link.port, link.code, link.fingerprint)
        }
    }

    Column(
        Modifier.fillMaxSize().background(p.bg).statusBarsPadding().navigationBarsPadding().imePadding()
            .verticalScroll(rememberScrollState()).padding(start = 20.dp, end = 20.dp, top = 24.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Column {
            Txt("Pair with your PC", Modifier.padding(bottom = 8.dp), size = 32.sp, weight = FontWeight.Bold)
            Txt(
                if (manual) "On your PC open Bridge, choose Pair a device, then Pair a phone. Type the PC's address (shown as Phone address in Settings on the PC) and the code shown there."
                else "On your PC open Bridge, choose Pair a device, then Pair a phone. Then point this camera at the QR code.",
                color = p.label2, size = 15.sp,
            )
        }

        if (!manual) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                Box(Modifier.size(296.dp).clip(RoundedCornerShape(24.dp)).background(Color(0xFF15171C))) {
                    if (granted) {
                        QrScanner(::scanned, Modifier.fillMaxSize())
                    } else {
                        Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
                            Txt("Bridge needs the camera to read the QR code.", color = Color(0xFFD6D8DE), size = 14.sp, align = TextAlign.Center)
                            Box(Modifier.height(12.dp))
                            Pill("Allow camera", { permission.launch(Manifest.permission.CAMERA) }, primary = true)
                        }
                    }
                    Box(
                        Modifier.fillMaxSize().drawBehind {
                            // the four corners of the frame
                            val inset = 28.dp.toPx()
                            val len = 48.dp.toPx()
                            val stroke = Stroke(5.dp.toPx(), cap = StrokeCap.Round)
                            val w = size.width
                            val h = size.height
                            for ((x, y, dx, dy) in listOf(listOf(inset, inset, 1f, 1f), listOf(w - inset, inset, -1f, 1f), listOf(inset, h - inset, 1f, -1f), listOf(w - inset, h - inset, -1f, -1f))) {
                                drawLine(Color.White, Offset(x, y), Offset(x + dx * len, y), stroke.width, StrokeCap.Round)
                                drawLine(Color.White, Offset(x, y), Offset(x, y + dy * len), stroke.width, StrokeCap.Round)
                            }
                        },
                    )
                    Txt(
                        if (busy) "Pairing..." else "Looking for a code...",
                        Modifier.align(Alignment.BottomCenter).padding(bottom = 12.dp),
                        color = Color(0xFFD6D8DE), size = 13.sp,
                    )
                }
            }
        } else {
            Panel {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Field("PC address", address, "192.168.1.20:8443") { address = it }
                    Field("Code", code, "ABCD EFGH") { code = it }
                }
            }
            Footnote("Without the QR code this phone cannot check the PC's certificate on its own. After pairing it shows a fingerprint to compare with the one on the PC.")
        }

        error?.let { message ->
            Txt(message, Modifier.padding(horizontal = 4.dp), color = p.red, size = 14.sp)
        }

        Panel {
            Row(Modifier.padding(horizontal = 16.dp, vertical = 14.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
                Icn(R.drawable.ic_lock, p.tintText, 22.dp, Modifier.padding(top = 1.dp))
                Txt("Stays on your network. The QR code carries your PC's key, so this phone only ever talks to that PC.", color = p.label2, size = 14.sp)
            }
        }

        if (manual) {
            Pill(
                label = if (busy) "Pairing..." else "Pair",
                onClick = {
                    val target = parseAddress(address)
                    if (target == null) error = "That address does not look right. Use something like 192.168.1.20:8443." else attempt(target.first, target.second, code, null)
                },
                modifier = Modifier.fillMaxWidth(),
                primary = true,
                enabled = !busy && address.isNotBlank() && code.isNotBlank(),
                height = 48.dp,
            )
            Pill("Scan the QR code instead", { manual = false; error = null }, Modifier.fillMaxWidth(), height = 48.dp)
        } else {
            if (error != null) Pill("Scan again", { error = null; lastTried = null }, Modifier.fillMaxWidth(), primary = true, height = 48.dp)
            Pill("Enter code instead", { manual = true; error = null }, Modifier.fillMaxWidth(), height = 48.dp)
        }
        if (canLeave) {
            TextAction("Not now", onLeave, Modifier.fillMaxWidth())
        }
    }

    verify?.let { pc ->
        AlertDialog(
            onDismissRequest = {},
            containerColor = p.card,
            title = { Text("Check the PC", color = p.label) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("This phone paired without the QR code. Open Settings in Bridge on the PC and look at the certificate fingerprint. It must start with:", color = p.label2)
                    Text(shortFingerprint(pc.fingerprint), color = p.label, fontFamily = FontFamily.Monospace, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                }
            },
            confirmButton = { TextAction("It matches", { verify = null; onDone() }) },
            dismissButton = { TextAction("It differs", { Bridge.unpair(pc); verify = null }, color = p.red) },
        )
    }
}

@Composable
private fun Field(label: String, value: String, hint: String, onChange: (String) -> Unit) {
    val p = LocalPalette.current
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Txt(label, color = p.label2, size = 13.sp)
        BasicTextField(
            value = value,
            onValueChange = onChange,
            singleLine = true,
            textStyle = TextStyle(color = p.label, fontSize = 16.sp),
            cursorBrush = SolidColor(p.tint),
            modifier = Modifier.fillMaxWidth(),
            decorationBox = { field ->
                Box(Modifier.fillMaxWidth().height(44.dp).clip(RoundedCornerShape(8.dp)).background(p.field).padding(horizontal = 12.dp), contentAlignment = Alignment.CenterStart) {
                    if (value.isEmpty()) Txt(hint, color = p.label2)
                    field()
                }
            },
        )
    }
}

/** The camera, looking for QR codes. */
@Composable
private fun QrScanner(onCode: (String) -> Unit, modifier: Modifier) {
    val context = LocalContext.current
    val owner = context as LifecycleOwner
    val latest by rememberUpdatedState(onCode)
    val executor = remember { Executors.newSingleThreadExecutor() }
    var provider by remember { mutableStateOf<ProcessCameraProvider?>(null) }
    DisposableEffect(Unit) {
        onDispose {
            provider?.unbindAll()
            executor.shutdown()
        }
    }
    AndroidView(
        modifier = modifier,
        factory = { ctx ->
            val view = PreviewView(ctx)
            view.implementationMode = PreviewView.ImplementationMode.COMPATIBLE // a texture, so the rounded corners clip it
            view.scaleType = PreviewView.ScaleType.FILL_CENTER
            val future = ProcessCameraProvider.getInstance(ctx)
            future.addListener({
                try {
                    val cameras = future.get()
                    provider = cameras
                    val preview = Preview.Builder().build()
                    preview.setSurfaceProvider(view.surfaceProvider)
                    val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
                    val scanner = BarcodeScanning.getClient(BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())
                    analysis.setAnalyzer(executor) { image -> readCodes(scanner, image) { latest(it) } }
                    cameras.unbindAll()
                    cameras.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
                } catch (e: Exception) {
                    // no camera available: the user can still type the code
                }
            }, ContextCompat.getMainExecutor(ctx))
            view
        },
    )
}

@OptIn(ExperimentalGetImage::class)
private fun readCodes(scanner: BarcodeScanner, proxy: ImageProxy, onCode: (String) -> Unit) {
    val image = proxy.image
    if (image == null) {
        proxy.close()
        return
    }
    scanner.process(InputImage.fromMediaImage(image, proxy.imageInfo.rotationDegrees))
        .addOnSuccessListener { codes -> codes.firstOrNull()?.rawValue?.let(onCode) }
        .addOnCompleteListener { proxy.close() }
}
