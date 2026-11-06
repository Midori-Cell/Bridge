package dev.bridge.phone

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            BridgeTheme {
                AppRoot()
            }
        }
    }

    // The conversation is only followed while the app is on screen.
    override fun onStart() {
        super.onStart()
        Bridge.setVisible(true)
    }

    override fun onStop() {
        Bridge.setVisible(false)
        super.onStop()
    }
}

enum class Tab(val label: String, val icon: Int) {
    Notes("Notes", R.drawable.ic_chat),
    Files("Files", R.drawable.ic_folder),
    Settings("Settings", R.drawable.ic_gear),
}

@Composable
fun AppRoot() {
    val p = LocalPalette.current
    val context = LocalContext.current
    val pcs by Bridge.pcs.collectAsState()
    val phones by Phones.peers.collectAsState()
    var tab by rememberSaveable { mutableStateOf(Tab.Notes) }
    // "home", "pc" (scan a PC's QR code), "phone" (phones nearby) or "code" (compare the code with a phone)
    var route by rememberSaveable { mutableStateOf("home") }
    var codeId by rememberSaveable { mutableStateOf<String?>(null) }
    var choosing by remember { mutableStateOf(false) }
    val notifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }

    // short messages from the background work ("unpaired", "no longer in Downloads", ...)
    LaunchedEffect(Unit) {
        Bridge.messages.collect { toast(context, it) }
    }
    // once a PC is paired, ask (one time) to show the transfer notification
    LaunchedEffect(pcs.isNotEmpty() || phones.isNotEmpty()) {
        if ((pcs.isNotEmpty() || phones.isNotEmpty()) && Build.VERSION.SDK_INT >= 33 && !Bridge.askedNotifications) {
            Bridge.askedNotifications = true
            if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
    }

    // the PC wants to send a file: ask, like the PC does
    val offers by Bridge.offers.collectAsState()
    offers.firstOrNull()?.let { offer ->
        AlertDialog(
            onDismissRequest = { Bridge.answer(offer, false) },
            containerColor = p.card,
            title = { Txt("Receive a file?", weight = FontWeight.SemiBold) },
            text = { Txt("${offer.pc.name} wants to send you ${offer.file.name} (${sizeText(offer.file.size)}). It is saved in Downloads/Bridge.", color = p.label2) },
            confirmButton = { TextAction("Accept", { Bridge.answer(offer, true) }) },
            dismissButton = { TextAction("Decline", { Bridge.answer(offer, false) }, color = p.red) },
        )
    }

    // another phone asks to pair, or wants to send a file: shown over whatever is on screen
    IncomingPairDialog()
    PhoneOfferDialog()

    if (route == "pc") {
        BackHandler { route = "home" }
        PairScreen(canLeave = true, onDone = { route = "home"; tab = Tab.Notes }, onLeave = { route = "home" })
        return
    }
    if (route == "phone") {
        BackHandler { route = "home" }
        NearbyScreen(onBack = { route = "home" }, onStarted = { id -> codeId = id; route = "code" })
        return
    }
    if (route == "code") {
        val id = codeId
        if (id != null) {
            BackHandler { Phones.dismissPair(id); route = "home" }
            CodeScreen(id, onClose = { route = "home"; tab = Tab.Notes })
            return
        }
    }
    if (pcs.isEmpty() && phones.isEmpty()) {
        WelcomeScreen(onPairPc = { route = "pc" }, onPairPhone = { route = "phone" })
        return
    }

    val keyboardOpen = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    Column(Modifier.fillMaxSize().background(p.bg).imePadding()) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            when (tab) {
                Tab.Notes -> HomeScreen(onPair = { choosing = true })
                Tab.Files -> FilesScreen()
                Tab.Settings -> SettingsScreen(onPair = { choosing = true })
            }
        }
        if (!keyboardOpen) {
            Row(
                Modifier.fillMaxWidth().background(p.side)
                    .drawBehind { drawLine(p.sep, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx()) }
                    .navigationBarsPadding().padding(horizontal = 8.dp),
            ) {
                Tab.entries.forEach { t ->
                    val selected = t == tab
                    val color = if (selected) p.tintText else p.label2
                    Column(
                        Modifier.weight(1f).height(60.dp).clickable(role = Role.Tab) { tab = t },
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.Center,
                    ) {
                        Icn(t.icon, color, 24.dp)
                        Txt(t.label, color = color, size = 12.sp, weight = if (selected) FontWeight.SemiBold else null)
                    }
                }
            }
        }
    }

    if (choosing) {
        PairChooseDialog(
            onPc = { choosing = false; route = "pc" },
            onPhone = { choosing = false; route = "phone" },
            onClose = { choosing = false },
        )
    }
}

private fun Modifier.heightIn48(): Modifier = this.then(Modifier.height(48.dp))
