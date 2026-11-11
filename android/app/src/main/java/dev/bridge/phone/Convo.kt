package dev.bridge.phone

import android.content.Context
import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import dev.bridge.phone.p2p.Peer

/**
 * The conversation that is open: with a PC or with another phone. The screens work with this, so the chat looks and
 * behaves the same for both; only where the messages go differs.
 */
class Convo(
    val id: String,
    val name: String,
    val phone: Boolean,
    val notes: List<Note>,
    val link: Link,
    /** Messages up to this id are hidden in the chat ("Clear chat" on this phone). */
    val hiddenUpTo: Long,
    val transfers: List<Transfer>,
    private val pc: Pc?,
    private val peer: Peer?,
) {
    suspend fun send(text: String, replyTo: String?): String? =
        if (peer != null) Phones.sendText(peer, text, replyTo) else Bridge.sendText(text, replyTo, pc)

    suspend fun delete(note: Note): String? =
        if (note.isOffer) {
            note.file?.let { Phones.answerPicture(it.id, false) } // "Decline" on a picture that was offered
            null
        } else if (peer != null) Phones.deleteNote(peer, note) else Bridge.deleteNote(note)

    fun clear() {
        if (peer != null) Phones.clearChat(peer) else if (pc != null) Bridge.clearChat(pc)
    }

    fun sendFiles(context: Context, uris: List<Uri>) {
        if (uris.isNotEmpty()) TransferService.sendFilesTo(context, uris, id)
    }

    fun tap(context: Context, note: Note, saved: Map<String, String>) {
        fileTap(context, note, pc, saved)
    }
}

/** The open conversation, or null while nothing is paired. */
@Composable
fun rememberConvo(): Convo? {
    val pc by Bridge.active.collectAsState()
    val peer by Phones.active.collectAsState()
    val pcNotes by Bridge.notes.collectAsState()
    val phoneNotes by Phones.notes.collectAsState()
    val pcLink by Bridge.link.collectAsState()
    val phoneLink by Phones.link.collectAsState()
    val pcCleared by Bridge.cleared.collectAsState()
    val phoneCleared by Phones.cleared.collectAsState()
    val pcTransfers by Bridge.transfers.collectAsState()
    val phoneTransfers by Phones.transfers.collectAsState()
    val transfers = pcTransfers + phoneTransfers
    val ph = peer
    val c = pc
    return when {
        ph != null -> Convo(ph.fp, ph.name, true, phoneNotes, phoneLink, phoneCleared[ph.fp] ?: 0L, transfers, null, ph)
        c != null -> Convo(c.id, c.name, false, pcNotes, pcLink, pcCleared[c.id] ?: 0L, transfers, c, null)
        else -> null
    }
}
