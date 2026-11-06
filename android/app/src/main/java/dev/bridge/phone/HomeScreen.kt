package dev.bridge.phone

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** Takes whatever scrolling the chat cannot use, so the page behind it does not move when the chat reaches its end. */
private val keepScrollInChat = object : NestedScrollConnection {
    override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset = available
    override suspend fun onPostFling(consumed: Velocity, available: Velocity): Velocity = available
}

/** Mockup N: the connection, a small chat with the PC, and the most recent items. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun HomeScreen(onPair: () -> Unit) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val convo = rememberConvo()
    val latest by rememberUpdatedState(convo)
    val notes = convo?.notes ?: emptyList()
    val transfers = convo?.transfers ?: emptyList()
    val saved by Bridge.saved.collectAsState()
    val time24 by Bridge.time24.collectAsState()
    var text by rememberSaveable { mutableStateOf("") }
    var replyTo by remember { mutableStateOf<Note?>(null) }
    var sending by remember { mutableStateOf(false) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        latest?.sendFiles(context, uris)
    }
    val listState = rememberLazyListState()
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val chatCard = remember { BringIntoViewRequester() }
    val keyboardOpen = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    // when the keyboard comes up, move the page so the whole chat card, buttons included, stays visible
    LaunchedEffect(keyboardOpen) {
        if (keyboardOpen) {
            delay(200)
            chatCard.bringIntoView()
        }
    }
    // a new message scrolls the chat to the bottom, unless the user has scrolled up to read
    val newest = notes.lastOrNull()?.id
    LaunchedEffect(newest) {
        if (listState.firstVisibleItemIndex <= 2) listState.animateScrollToItem(0)
    }
    val fadeTop by animateFloatAsState(if (listState.canScrollForward) 0.95f else 0f, label = "fadeTop")
    val fadeBottom by animateFloatAsState(if (listState.canScrollBackward) 0.95f else 0f, label = "fadeBottom")
    val current = convo ?: return
    // "Clear chat" deletes the older texts on this phone only; the files below are not affected
    val hiddenUpTo = current.hiddenUpTo
    val chat = notes.filter { it.pending || it.id > hiddenUpTo }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).statusBarsPadding().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Txt("Notes", Modifier.weight(1f).padding(start = 4.dp), size = 32.sp, weight = FontWeight.Bold)
            Box(
                Modifier.size(44.dp).clip(CircleShape).background(p.tint).clickable(onClickLabel = "Pair a device", role = Role.Button) { onPair() },
                contentAlignment = Alignment.Center,
            ) {
                Icn(R.drawable.ic_plus, Color.White, 22.dp)
            }
        }

        DeviceChips(current)

        // how the device is reached, and Clear chat at the right end of the same line
        val (linkColor, linkLabel) = when (current.link) {
            Link.Online -> p.green to (if (current.phone) "Connected directly · encrypted" else "Connected · encrypted")
            Link.Connecting -> p.label2 to "Connecting..."
            Link.Offline -> p.red to (if (current.phone) "Phone not reachable. Messages wait and are sent when it is back." else "PC not reachable. Messages wait and are sent when it is back.")
        }
        Row(Modifier.fillMaxWidth().padding(start = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Box(Modifier.size(9.dp).clip(CircleShape).background(linkColor))
            Txt(linkLabel, Modifier.weight(1f), color = linkColor, size = 13.sp, maxLines = 2)
            TextAction("Clear chat", { current.clear() })
        }

        // the chat: messages above, the note being written below
        Column(Modifier.fillMaxWidth().height(300.dp).bringIntoViewRequester(chatCard).clip(RoundedCornerShape(12.dp)).background(p.card)) {
            Box(
                Modifier.weight(1f).fillMaxWidth()
                    .clip(RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp))
                    .background(p.chat)
                    .nestedScroll(keepScrollInChat)
                    .topBorder(p.outline, 12.dp),
            ) {
                if (chat.isEmpty()) {
                    Txt("Nothing here yet.", Modifier.align(Alignment.Center), color = p.label2, size = 14.sp)
                }
                LazyColumn(
                    Modifier.fillMaxSize(),
                    state = listState,
                    reverseLayout = true,
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 10.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    val shown = chat.asReversed()
                    itemsIndexed(shown, key = { _, n -> n.id }) { i, note ->
                        // the list runs newest first, so the next item is the older message
                        val older = shown.getOrNull(i + 1)
                        val firstOfDay = note.ts > 0 && (older == null || older.ts <= 0 || dayOf(older.ts) != dayOf(note.ts))
                        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            if (firstOfDay) DayLine(dayLabel(dayOf(note.ts)))
                            Bubble(
                                note = note,
                                state = fileState(note, transfers, saved),
                                time = if (note.pending) "Waiting to send" else if (note.ts > 0) timeText(note.ts, time24) else "",
                                onTap = { current.tap(context, note, saved) },
                                onCopy = { copyText(context, note.text.orEmpty()) },
                                onReply = { replyTo = note },
                                onDelete = {
                                    scope.launch {
                                        current.delete(note)?.let { toast(context, it) }
                                    }
                                },
                            )
                        }
                    }
                }
                // soft edges where messages are cut off, as on the PC: the chat goes on there
                Box(Modifier.align(Alignment.TopCenter).fillMaxWidth().height(22.dp).background(Brush.verticalGradient(listOf(p.chat.copy(alpha = fadeTop), Color.Transparent))))
                Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(22.dp).background(Brush.verticalGradient(listOf(Color.Transparent, p.chat.copy(alpha = fadeBottom)))))
            }
            Column(Modifier.padding(start = 12.dp, end = 12.dp, top = 8.dp, bottom = 10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                replyTo?.let { quoted ->
                    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(p.field).padding(start = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Txt("Replying to: ${quoted.text.orEmpty()}", Modifier.weight(1f), color = p.label2, size = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Box(Modifier.size(44.dp).clickable { replyTo = null }, contentAlignment = Alignment.Center) {
                            Icn(R.drawable.ic_close, p.label2, 18.dp)
                        }
                    }
                }
                BasicTextField(
                    value = text,
                    onValueChange = { text = it },
                    textStyle = TextStyle(color = p.label, fontSize = 16.sp),
                    cursorBrush = SolidColor(p.tint),
                    maxLines = 4,
                    modifier = Modifier.fillMaxWidth(),
                    decorationBox = { field ->
                        Box(
                            Modifier.fillMaxWidth().heightIn(min = 40.dp).clip(RoundedCornerShape(8.dp)).background(p.field).padding(horizontal = 12.dp, vertical = 10.dp),
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            if (text.isEmpty()) Txt("Write a note for ${current.name}", color = p.label2)
                            field()
                        }
                    },
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Pill("Files", onClick = { picker.launch(arrayOf("*/*")) })
                    Pill("Clipboard", onClick = {
                        val clip = clipboardText(context)
                        if (clip == null) toast(context, "The clipboard is empty.") else text = if (text.isEmpty()) clip else text + clip
                    })
                    Spacer(Modifier.weight(1f))
                    Pill("Send", primary = true, padding = 24.dp, enabled = text.isNotBlank() && !sending, onClick = {
                        sending = true
                        focus.clearFocus()
                        keyboard?.hide()
                        scope.launch {
                            val error = current.send(text, replyTo?.mid)
                            sending = false
                            if (error == null) {
                                text = ""
                                replyTo = null
                                listState.scrollToItem(0)
                            } else {
                                toast(context, error)
                            }
                        }
                    })
                }
            }
        }

        SectionLabel("FILES")
        Panel {
            val recent = notes.filter { it.isFile }.takeLast(3).asReversed()
            if (recent.isEmpty()) Txt("Files you exchange with ${current.name} show up here.", Modifier.padding(16.dp), color = p.label2, size = 14.sp)
            recent.forEachIndexed { i, note ->
                if (i > 0) Sep()
                RecentRow(note, current.name, fileState(note, transfers, saved), onTap = { current.tap(context, note, saved) }, onCopy = { copyText(context, note.text.orEmpty()) })
            }
        }
        Spacer(Modifier.height(16.dp))
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Bubble(note: Note, state: FileState, time: String, onTap: () -> Unit, onCopy: () -> Unit, onReply: () -> Unit, onDelete: () -> Unit) {
    val p = LocalPalette.current
    val mine = !note.fromPc
    val background = if (mine) p.tint else p.bubble
    val foreground = if (mine) Color.White else p.label
    val quiet = if (mine) Color.White.copy(alpha = 0.85f) else p.label2
    val shape = RoundedCornerShape(topStart = 18.dp, topEnd = 18.dp, bottomStart = if (mine) 18.dp else 6.dp, bottomEnd = if (mine) 6.dp else 18.dp)
    var menu by remember { mutableStateOf(false) }
    // The menu opens where the finger is, so it is always on screen, even for a message that is longer than the chat.
    var menuAt by remember { mutableStateOf(IntOffset.Zero) }
    val haptic = LocalHapticFeedback.current
    val openMenu = rememberUpdatedState<(Offset) -> Unit> { at ->
        menuAt = IntOffset(at.x.roundToInt(), at.y.roundToInt())
        haptic.performHapticFeedback(HapticFeedbackType.LongPress)
        menu = true
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (mine) Arrangement.End else Arrangement.Start) {
        Box {
            Column(
                Modifier.widthIn(max = 270.dp).clip(shape).background(background)
                    .longPressAnywhere(openMenu)
                    .then(if (note.isFile) Modifier.clickable { onTap() } else Modifier)
                    .padding(horizontal = 14.dp, vertical = 7.dp),
            ) {
                note.reply?.let { quote ->
                    Txt("↩ ${quote.preview}", Modifier.padding(bottom = 3.dp), color = quiet, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                val file = note.file
                if (file != null) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        Box(
                            Modifier.size(30.dp).clip(RoundedCornerShape(8.dp)).background(if (mine) Color.White.copy(alpha = 0.22f) else p.field),
                            contentAlignment = Alignment.Center,
                        ) {
                            Icn(R.drawable.ic_file, foreground, 16.dp)
                        }
                        Column(Modifier.weight(1f, fill = false)) {
                            Txt(file.name, color = foreground, weight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Txt(state.status, color = quiet, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                } else {
                    Text(linkify(note.text.orEmpty(), if (mine) Color.White else p.tintText), color = foreground, fontSize = 16.sp)
                }
                // small, and a little lower and further right than the text, into the padding of the bubble
                if (time.isNotEmpty()) {
                    Txt(time, Modifier.align(Alignment.End).offset(x = 3.dp, y = 2.dp), color = quiet, size = 11.sp)
                }
            }
            Box(Modifier.offset { menuAt }) {
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, modifier = Modifier.background(p.card)) {
                    if (note.isFile) {
                        state.action?.let { action ->
                            MenuRow(action, R.drawable.ic_file, p.label) { menu = false; onTap() }
                        }
                    } else {
                        MenuRow("Copy", R.drawable.ic_copy, p.label) { menu = false; onCopy() }
                        if (!note.pending) MenuRow("Reply", R.drawable.ic_reply, p.label) { menu = false; onReply() }
                    }
                    MenuRow(if (note.pending) "Cancel sending" else "Delete", R.drawable.ic_trash, p.red) { menu = false; onDelete() }
                }
            }
        }
    }
}

/** A thin line across the chat with the day in the middle. */
@Composable
private fun DayLine(label: String) {
    val p = LocalPalette.current
    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Box(Modifier.weight(1f).height(1.dp).background(p.outline.copy(alpha = 0.45f)))
        Txt(label, color = p.label2, size = 12.sp, weight = FontWeight.Medium)
        Box(Modifier.weight(1f).height(1.dp).background(p.outline.copy(alpha = 0.45f)))
    }
}

@Composable
private fun MenuRow(label: String, icon: Int, color: Color, onClick: () -> Unit) {
    DropdownMenuItem(
        text = { Text(label, color = color) },
        leadingIcon = { Icn(icon, color, 20.dp) },
        onClick = onClick,
    )
}

@Composable
private fun RecentRow(note: Note, deviceName: String, state: FileState, onTap: () -> Unit, onCopy: () -> Unit) {
    val p = LocalPalette.current
    val who = if (note.fromPc) deviceName else "You"
    val file = note.file
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        if (file != null) {
            Box(Modifier.size(40.dp).clip(RoundedCornerShape(8.dp)).background(p.field), contentAlignment = Alignment.Center) {
                Icn(R.drawable.ic_file, p.label, 20.dp)
            }
        }
        Column(Modifier.weight(1f).padding(vertical = 6.dp)) {
            Txt("$who · ${whenText(note.ts)}", color = p.label2, size = 13.sp)
            if (file != null) {
                Txt(file.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Txt(state.status, color = p.label2, size = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            } else {
                Txt(note.text.orEmpty(), maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
        if (file != null) {
            state.action?.let { TextAction(it, onTap) }
        } else {
            TextAction("Copy", onCopy)
        }
    }
}

/** DEVICES: every paired PC and phone as a chip; the one that is open is highlighted. */
@Composable
private fun DeviceChips(current: Convo) {
    val p = LocalPalette.current
    val pcs by Bridge.pcs.collectAsState()
    val peers by Phones.peers.collectAsState()
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        SectionLabel("DEVICES")
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            pcs.forEach { pc ->
                val selected = !current.phone && current.id == pc.id
                val status = if (selected) when (current.link) { Link.Online -> "Online"; Link.Connecting -> "Connecting"; Link.Offline -> "Not reachable" } else "PC"
                Chip(R.drawable.ic_monitor, pc.name, status, selected, current.link == Link.Online && selected) { Bridge.select(pc) }
            }
            peers.forEach { ph ->
                val selected = current.phone && current.id == ph.fp
                Chip(R.drawable.ic_phone, ph.name, if (ph.online) "Online" else "Not reachable", selected, ph.online) { Phones.select(ph) }
            }
        }
    }
}

@Composable
private fun Chip(icon: Int, name: String, status: String, selected: Boolean, online: Boolean, onClick: () -> Unit) {
    val p = LocalPalette.current
    Column(
        Modifier.clip(RoundedCornerShape(14.dp)).background(if (selected) p.tint.copy(alpha = 0.16f) else p.card)
            .clickable(role = Role.RadioButton, onClick = onClick).padding(horizontal = 14.dp, vertical = 9.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icn(icon, if (selected) p.tintText else p.label, 18.dp)
            Txt(name, color = if (selected) p.tintText else p.label, weight = if (selected) FontWeight.SemiBold else null, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 150.dp))
        }
        Txt(status, Modifier.padding(start = 26.dp), color = if (online) p.green else p.label2, size = 12.sp)
    }
}
