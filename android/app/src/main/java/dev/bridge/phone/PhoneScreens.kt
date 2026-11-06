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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.bridge.phone.p2p.PairState
import kotlinx.coroutines.launch

/** "host" or "host:port" as typed by the user; the usual port is used when none is given. */
private fun parsePhoneAddress(text: String): Pair<String, Int>? {
    val typed = text.trim().removePrefix("https://").trimEnd('/')
    if (typed.isEmpty()) return null
    val i = typed.lastIndexOf(':')
    if (i > 0 && !typed.contains("::") && !typed.startsWith("[")) {
        val port = typed.substring(i + 1).toIntOrNull() ?: return null
        return typed.substring(0, i) to port
    }
    return typed to Phones.DEFAULT_PORT
}

/** A row with an icon, a title and a line under it, as in the mockups. */
@Composable
private fun ChoiceRow(icon: Int, title: String, text: String, onClick: () -> Unit) {
    val p = LocalPalette.current
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).clickable(role = Role.Button, onClick = onClick).padding(vertical = 10.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(Modifier.size(44.dp).clip(RoundedCornerShape(12.dp)).background(p.field), contentAlignment = Alignment.Center) {
            Icn(icon, p.label, 24.dp)
        }
        Column(Modifier.weight(1f)) {
            Txt(title, weight = FontWeight.SemiBold)
            Txt(text, color = p.label2, size = 14.sp)
        }
    }
}

@Composable
private fun LockLine(text: String) {
    val p = LocalPalette.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.Top) {
        Icn(R.drawable.ic_lock, p.tintText, 20.dp, Modifier.padding(top = 1.dp))
        Txt(text, color = p.label2, size = 13.sp)
    }
}

/** The first start: pair a PC or a phone. */
@Composable
fun WelcomeScreen(onPairPc: () -> Unit, onPairPhone: () -> Unit) {
    val p = LocalPalette.current
    Column(
        Modifier.fillMaxSize().background(p.bg).statusBarsPadding().navigationBarsPadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Spacer(Modifier.height(24.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(36.dp, Alignment.CenterHorizontally)) {
            Icn(R.drawable.ic_monitor, p.label, 40.dp)
            Icn(R.drawable.ic_phone, p.label, 40.dp)
        }
        Txt("Welcome to Bridge", Modifier.fillMaxWidth(), size = 32.sp, weight = FontWeight.Bold, align = TextAlign.Center)
        Txt(
            "Send text and files between this phone, your computers and other phones. It all stays on your own network.",
            Modifier.fillMaxWidth(), color = p.label2, size = 16.sp, align = TextAlign.Center,
        )
        Spacer(Modifier.height(16.dp))
        ChoiceRow(R.drawable.ic_monitor, "Pair a PC", "Scan the QR code that Bridge shows on your computer", onPairPc)
        ChoiceRow(R.drawable.ic_phone, "Pair a phone", "Connect to another phone with Bridge on the same Wi-Fi", onPairPhone)
        Spacer(Modifier.height(24.dp))
        LockLine("No account, no internet. Connections between your devices are encrypted.")
    }
}

/** The "+" button: what do you want to pair? */
@Composable
fun PairChooseDialog(onPc: () -> Unit, onPhone: () -> Unit, onClose: () -> Unit) {
    val p = LocalPalette.current
    AlertDialog(
        onDismissRequest = onClose,
        containerColor = p.card,
        title = { Txt("Pair a device", weight = FontWeight.SemiBold, size = 22.sp) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                ChoiceRow(R.drawable.ic_monitor, "Pair a PC", "Scan the QR code that Bridge shows on your computer", onPc)
                ChoiceRow(R.drawable.ic_phone, "Pair a phone", "Connect to another phone with Bridge on the same Wi-Fi", onPhone)
                Spacer(Modifier.height(4.dp))
                Txt("A pairing is only done once. After that the devices find each other on their own, and every connection between them is encrypted.", color = p.label2, size = 13.sp)
            }
        },
        confirmButton = { TextAction("Cancel", onClose) },
    )
}

/** Pair a phone: the phones that are open in Bridge on this Wi-Fi, or an address typed by hand. */
@Composable
fun NearbyScreen(onBack: () -> Unit, onStarted: (String) -> Unit) {
    val p = LocalPalette.current
    val scope = rememberCoroutineScope()
    val nearby by Phones.nearby.collectAsState()
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var byAddress by remember { mutableStateOf(false) }

    fun start(host: String, port: Int) {
        if (busy) return
        busy = true
        error = null
        scope.launch {
            val (message, id) = Phones.pair(host, port)
            busy = false
            if (id != null) onStarted(id) else error = message
        }
    }

    Column(
        Modifier.fillMaxSize().background(p.bg).statusBarsPadding().navigationBarsPadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(Modifier.size(44.dp).clickable(role = Role.Button, onClickLabel = "Back", onClick = onBack), contentAlignment = Alignment.CenterStart) {
            Icn(R.drawable.ic_back, p.label, 24.dp)
        }
        Txt("Pair a phone", size = 32.sp, weight = FontWeight.Bold)
        Txt("On the other phone, open Bridge and choose Pair a device > Pair a phone. Both phones must be on the same Wi-Fi. It shows up here.", color = p.label2, size = 15.sp)

        SectionLabel("NEARBY")
        Panel {
            nearby.forEachIndexed { i, ph ->
                if (i > 0) Sep()
                Row(
                    Modifier.fillMaxWidth().clickable(enabled = !busy, role = Role.Button) { start(ph.host, ph.port) }.padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Icn(R.drawable.ic_phone, p.label, 24.dp)
                    Column(Modifier.weight(1f)) {
                        Txt(ph.name, weight = FontWeight.SemiBold)
                        Txt("Bridge is open, same Wi-Fi", color = p.label2, size = 13.sp)
                    }
                }
            }
            if (nearby.isNotEmpty()) Sep()
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                CircularProgressIndicator(Modifier.size(20.dp), color = p.label2, strokeWidth = 2.dp)
                Txt(if (busy) "Asking the phone..." else "Looking for more phones...", color = p.label2)
            }
        }
        error?.let { Txt(it, Modifier.padding(horizontal = 4.dp), color = p.red, size = 14.sp) }
        TextAction("Pair by address...", { byAddress = true })
        Txt(
            "A phone does not show up? Some Wi-Fi networks stop devices from finding each other. Choose Pair by address and type the address that the other phone shows in its Settings.",
            Modifier.padding(horizontal = 4.dp), color = p.label2, size = 13.sp,
        )
        Spacer(Modifier.height(12.dp))
        LockLine("The other phone only learns this phone's name. It does not see your PCs, your messages or your files.")
    }

    if (byAddress) {
        var typed by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { byAddress = false },
            containerColor = p.card,
            title = { Text("Address of the other phone", color = p.label) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextField(value = typed, onValueChange = { typed = it.take(60) }, singleLine = true, placeholder = { Text("192.168.1.23") })
                    Text("Open Settings in Bridge on the other phone. It shows its address there.", color = p.label2, fontSize = 13.sp)
                }
            },
            confirmButton = {
                TextAction("Pair", {
                    val target = parsePhoneAddress(typed)
                    if (target == null) {
                        error = "That address does not look right. Use something like 192.168.1.23."
                    } else {
                        start(target.first, target.second)
                    }
                    byAddress = false
                })
            },
            dismissButton = { TextAction("Cancel", { byAddress = false }) },
        )
    }
}

private fun codeLook() = FontFamily.Monospace

/** Both users compare the six digits; when both agree, the phones trust each other. */
@Composable
fun CodeScreen(pairId: String, onClose: () -> Unit) {
    val p = LocalPalette.current
    val pairs by Phones.outPairs.collectAsState()
    val pair = pairs.firstOrNull { it.id == pairId }

    // a finished pairing opens the chat with the new phone
    LaunchedEffect(pair?.state) {
        if (pair != null && pair.state == PairState.Done) {
            Phones.openPeer(pair.fp)
            Phones.dismissPair(pair.id)
            onClose()
        }
    }
    if (pair == null) {
        LaunchedEffect(Unit) { onClose() }
        return
    }

    Column(
        Modifier.fillMaxSize().background(p.bg).statusBarsPadding().navigationBarsPadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(Modifier.size(44.dp).clickable(role = Role.Button, onClickLabel = "Back") { Phones.dismissPair(pair.id); onClose() }, contentAlignment = Alignment.CenterStart) {
            Icn(R.drawable.ic_back, p.label, 24.dp)
        }
        Txt("Pair with ${pair.peerName}", size = 32.sp, weight = FontWeight.Bold)
        Txt("${pair.peerName} shows a code too. Check that both phones show the same six digits.", color = p.label2, size = 15.sp)
        Spacer(Modifier.height(16.dp))
        Txt("CODE ON THIS PHONE", Modifier.fillMaxWidth(), color = p.label2, size = 13.sp, align = TextAlign.Center)
        Text(pair.code, Modifier.fillMaxWidth(), color = p.label, fontSize = 48.sp, fontWeight = FontWeight.Bold, fontFamily = codeLook(), textAlign = TextAlign.Center, letterSpacing = 4.sp)
        Spacer(Modifier.height(8.dp))

        when (pair.state) {
            PairState.Declined -> {
                Txt("${pair.peerName} declined.", Modifier.fillMaxWidth(), color = p.red, align = TextAlign.Center)
                Pill("Close", { Phones.dismissPair(pair.id); onClose() }, Modifier.fillMaxWidth(), primary = true, height = 48.dp)
            }
            PairState.Failed -> {
                Txt(pair.error ?: "Pairing did not work.", Modifier.fillMaxWidth(), color = p.red, align = TextAlign.Center)
                Pill("Close", { Phones.dismissPair(pair.id); onClose() }, Modifier.fillMaxWidth(), primary = true, height = 48.dp)
            }
            else -> {
                if (pair.confirmed) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.size(20.dp), color = p.label2, strokeWidth = 2.dp)
                        Txt("Waiting for ${pair.peerName}...", color = p.label2)
                    }
                } else {
                    Pill("The codes match", { Phones.confirmPair(pair.id) }, Modifier.fillMaxWidth(), primary = true, height = 48.dp)
                    if (pair.state == PairState.Waiting) {
                        Txt("${pair.peerName} has not answered yet. Its screen asks to compare the same code.", Modifier.fillMaxWidth(), color = p.label2, size = 13.sp, align = TextAlign.Center)
                    }
                }
                Pill("Cancel", { Phones.cancelPair(pair.id); onClose() }, Modifier.fillMaxWidth(), height = 48.dp)
            }
        }
        Txt("Only continue if the codes are exactly the same. If they are different, tap Cancel: somebody on the network may be in the way.", color = p.label2, size = 13.sp)
        Spacer(Modifier.height(12.dp))
        Panel {
            Column(Modifier.padding(horizontal = 16.dp, vertical = 14.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Txt("Encrypted connection to ${pair.peerName}", weight = FontWeight.SemiBold, size = 14.sp)
                Txt(
                    "Each phone proves who it is with its own certificate. The code shows that nobody sits between you. ${pair.peerName}'s certificate: ${pair.fp.take(8).chunked(4).joinToString(" ")}",
                    color = p.label2, size = 13.sp,
                )
            }
        }
    }
}

/** Another phone asks to pair: shown over everything, with the same code the other phone shows. */
@Composable
fun IncomingPairDialog() {
    val p = LocalPalette.current
    val requests by Phones.requests.collectAsState()
    val request = requests.firstOrNull() ?: return
    Dialog(onDismissRequest = {}, properties = DialogProperties(usePlatformDefaultWidth = false, dismissOnBackPress = false, dismissOnClickOutside = false)) {
        Column(
            Modifier.fillMaxSize().background(p.bg).statusBarsPadding().navigationBarsPadding().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(Modifier.height(48.dp))
            Icn(R.drawable.ic_phone, p.label, 40.dp)
            Txt("${request.name} wants to pair", Modifier.fillMaxWidth(), size = 24.sp, weight = FontWeight.Bold, align = TextAlign.Center)
            Txt("If you did not start this on the other phone, tap Decline.", Modifier.fillMaxWidth(), color = p.label2, size = 15.sp, align = TextAlign.Center)
            Spacer(Modifier.height(16.dp))
            Txt("CODE ON THIS PHONE", color = p.label2, size = 13.sp)
            Text(request.code, color = p.label, fontSize = 48.sp, fontWeight = FontWeight.Bold, fontFamily = codeLook(), letterSpacing = 4.sp)
            Txt("${request.name} must show the same code.", color = p.label2, size = 14.sp)
            Spacer(Modifier.height(12.dp))
            if (request.accepted) {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(20.dp), color = p.label2, strokeWidth = 2.dp)
                    Txt("Waiting for ${request.name} to confirm...", color = p.label2)
                }
            } else {
                Pill("The codes match", { Phones.acceptRequest(request.id) }, Modifier.fillMaxWidth(), primary = true, height = 48.dp)
            }
            Pill("Decline", { Phones.declineRequest(request.id) }, Modifier.fillMaxWidth(), danger = true, height = 48.dp)
            Spacer(Modifier.height(12.dp))
            Txt(
                "Nothing is shared before you agree. After pairing, ${request.name} can send you notes and files. Remove it any time in Settings.",
                color = p.label2, size = 13.sp, align = TextAlign.Center,
            )
        }
    }
}

/** Another device wants to send a file: ask, like for a PC. */
@Composable
fun PhoneOfferDialog() {
    val p = LocalPalette.current
    val offers by Phones.offers.collectAsState()
    val offer = offers.firstOrNull() ?: return
    AlertDialog(
        onDismissRequest = { Phones.answer(offer, false) },
        containerColor = p.card,
        title = { Txt("Receive a file?", weight = FontWeight.SemiBold) },
        text = { Txt("${offer.peerName} wants to send you ${offer.name} (${sizeText(offer.size)}). It is saved in Downloads/Bridge.", color = p.label2) },
        confirmButton = { TextAction("Accept", { Phones.answer(offer, true) }) },
        dismissButton = { TextAction("Decline", { Phones.answer(offer, false) }, color = p.red) },
    )
}
