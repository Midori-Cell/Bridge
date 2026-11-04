package dev.bridge.phone

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Takes whatever scrolling the chat cannot use, so the page behind it does not move when the chat reaches its end. */
private val keepScrollInChat = object : NestedScrollConnection {
    override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset = available
    override suspend fun onPostFling(consumed: Velocity, available: Velocity): Velocity = available
}

/** Mockup N: the connection, a small chat with the PC, and the most recent items. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun HomeScreen(onChange: () -> Unit) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val pc by Bridge.active.collectAsState()
    val link by Bridge.link.collectAsState()
    val notes by Bridge.notes.collectAsState()
    val transfers by Bridge.transfers.collectAsState()
    val saved by Bridge.saved.collectAsState()
    var text by rememberSaveable { mutableStateOf("") }
    var replyTo by remember { mutableStateOf<Note?>(null) }
    var sending by remember { mutableStateOf(false) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        val target = Bridge.active.value
        if (target != null && uris.isNotEmpty()) TransferService.sendFiles(context, uris, target)
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
    val current = pc ?: return

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).statusBarsPadding().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Txt("Notes", Modifier.padding(start = 4.dp, top = 8.dp), size = 32.sp, weight = FontWeight.Bold)

        // connection
        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(p.card).padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(p.field), contentAlignment = Alignment.Center) {
                Icn(R.drawable.ic_monitor, p.label, 22.dp)
            }
            Column(Modifier.weight(1f)) {
                Txt(current.name, weight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                val (color, label) = when (link) {
                    Link.Online -> p.green to "Connected · encrypted"
                    Link.Connecting -> p.label2 to "Connecting..."
                    Link.Offline -> p.red to "PC not reachable"
                }
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Box(Modifier.size(9.dp).clip(CircleShape).background(color))
                    Txt(label, color = color, size = 13.sp)
                }
            }
            TextAction("Change", onChange)
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
                if (notes.isEmpty()) {
                    Txt("Nothing here yet.", Modifier.align(Alignment.Center), color = p.label2, size = 14.sp)
                }
                LazyColumn(
                    Modifier.fillMaxSize(),
                    state = listState,
                    reverseLayout = true,
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 10.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    items(notes.asReversed(), key = { it.id }) { note ->
                        Bubble(
                            note = note,
                            state = fileState(note, transfers, saved),
                            onTap = { fileTap(context, note, current, saved) },
                            onCopy = { copyText(context, note.text.orEmpty()) },
                            onReply = { replyTo = note },
                            onDelete = {
                                scope.launch {
                                    Bridge.deleteNote(note)?.let { toast(context, it) }
                                }
                            },
                        )
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
                            val error = Bridge.sendText(text, replyTo?.mid, current)
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

        SectionLabel("RECENT")
        Panel {
            val recent = notes.takeLast(3).asReversed()
            if (recent.isEmpty()) Txt("Messages and files you exchange with ${current.name} show up here.", Modifier.padding(16.dp), color = p.label2, size = 14.sp)
            recent.forEachIndexed { i, note ->
                if (i > 0) Sep()
                RecentRow(note, current, fileState(note, transfers, saved), onTap = { fileTap(context, note, current, saved) }, onCopy = { copyText(context, note.text.orEmpty()) })
            }
        }
        Spacer(Modifier.height(16.dp))
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Bubble(note: Note, state: FileState, onTap: () -> Unit, onCopy: () -> Unit, onReply: () -> Unit, onDelete: () -> Unit) {
    val p = LocalPalette.current
    val mine = !note.fromPc
    val background = if (mine) p.tint else p.bubble
    val foreground = if (mine) Color.White else p.label
    val quiet = if (mine) Color.White.copy(alpha = 0.85f) else p.label2
    val shape = RoundedCornerShape(topStart = 18.dp, topEnd = 18.dp, bottomStart = if (mine) 18.dp else 6.dp, bottomEnd = if (mine) 6.dp else 18.dp)
    var menu by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (mine) Arrangement.End else Arrangement.Start) {
        Box {
            Column(
                Modifier.widthIn(max = 270.dp).clip(shape).background(background)
                    .combinedClickable(onClick = { if (note.isFile) onTap() }, onLongClick = { menu = true })
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
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, modifier = Modifier.background(p.card)) {
                if (note.isFile) {
                    state.action?.let { action ->
                        MenuRow(action, R.drawable.ic_file, p.label) { menu = false; onTap() }
                    }
                } else {
                    MenuRow("Copy", R.drawable.ic_copy, p.label) { menu = false; onCopy() }
                    MenuRow("Reply", R.drawable.ic_reply, p.label) { menu = false; onReply() }
                }
                MenuRow("Delete", R.drawable.ic_trash, p.red) { menu = false; onDelete() }
            }
        }
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
private fun RecentRow(note: Note, pc: Pc, state: FileState, onTap: () -> Unit, onCopy: () -> Unit) {
    val p = LocalPalette.current
    val who = if (note.fromPc) pc.name else "You"
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
