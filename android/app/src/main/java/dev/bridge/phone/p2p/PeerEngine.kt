package dev.bridge.phone.p2p

import dev.bridge.phone.FileRef
import dev.bridge.phone.History
import dev.bridge.phone.Note
import dev.bridge.phone.Reply
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.io.RandomAccessFile
import java.net.InetAddress
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/** Something to send: a name, a size and a way to read it from any position. */
interface FileSource {
    val name: String
    val size: Long
    fun open(offset: Long): InputStream
}

class FileOnDisk(private val file: File) : FileSource {
    override val name: String get() = file.name
    override val size: Long get() = file.length()
    override fun open(offset: Long): InputStream = java.io.FileInputStream(file).also { it.skipNBytes(offset) }
}

/** Where a paired device was last seen on the network (discovery), as host and port. */
fun interface Finder {
    fun addresses(fp: String): List<Pair<String, Int>>
}

/** An error whose text can be shown to the user as it is. */
class PeerFailure(message: String) : Exception(message)

/** The other device cannot be reached right now (off, away, other network). */
class Offline : IOException("offline")

data class Peer(val fp: String, val name: String, val online: Boolean)

enum class PairState { Waiting, PeerAccepted, Done, Declined, Failed }

/** This phone asked another device to pair; both users must see the same `code`. */
data class OutPair(val id: String, val peerName: String, val code: String, val state: PairState, val error: String?, val fp: String = "", val confirmed: Boolean = false)

/** Another device asks to pair with this phone. `accepted`: this user compared the code and agreed. */
data class InPair(val id: String, val name: String, val code: String, val accepted: Boolean)

enum class XState { Offered, Queued, Active, Done, Declined, Cancelled, Failed }

/** A file on its way to or from a paired device. */
data class Xfer(
    val id: String,
    val peerFp: String,
    val peerName: String,
    val name: String,
    val size: Long,
    val done: Long,
    val incoming: Boolean,
    val state: XState,
    val error: String? = null,
) {
    val running: Boolean get() = state == XState.Offered || state == XState.Queued || state == XState.Active
}

/**
 * Phone to phone (and phone to PC): the same protocol that two PCs speak, see docs/PROTOCOL.md. Every connection is
 * mutual TLS. A device is trusted because its certificate fingerprint was compared by both users when pairing; after
 * that only that exact certificate is accepted, in both directions.
 *
 * The engine knows nothing about Android: files come in as [FileSource], received files land in `dir/inbox`, and the
 * app is told through [onChange] (anything changed: re-read what you show) and [onFileReceived].
 */
class PeerEngine(
    private val dir: File,
    private val identity: Identity,
    private val name: () -> String,
    private val finder: Finder = Finder { emptyList() },
    private val openRef: (String) -> FileSource? = { null },
    private val autoAccept: () -> Boolean = { false },
    private val onChange: () -> Unit = {},
    private val onFileReceived: (fp: String, note: Note, file: File) -> Unit = { _, _, _ -> },
    private val bind: InetAddress? = null,
    private val wantedPort: Int = 0,
    private val maxFileSize: Long = 20L * 1024 * 1024 * 1024,
) {
    companion object {
        const val PROTOCOL = 1
        const val MAX_TEXT_CHARS = 100_000
        const val PENDING_BASE = 1L shl 40
        private const val UPLOAD_CHUNK = 4 * 1024 * 1024
        private const val PAIR_TTL_MS = 120_000L
        private const val MAX_PENDING_PAIRS = 4
        private const val MAX_RETRIES = 6
        private const val ONLINE_MS = 60_000L
        private const val OFFER_TTL_MS = 30 * 60_000L
        private const val MAX_OPEN_PER_PEER = 8

        fun sha256(vararg parts: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").also { d -> parts.forEach { d.update(it) } }.digest()
        fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }
        fun unhex(s: String): ByteArray? = if (s.length % 2 != 0 || !s.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) null else ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

        fun commitment(na: ByteArray) = sha256("bridge-commit-v1|".toByteArray(), na)

        /** The six digits both users compare. Depends on both fingerprints, who asked, and both nonces. */
        fun pairCode(initFp: String, respFp: String, na: ByteArray, nb: ByteArray): String {
            val d = sha256("bridge-pair-v1|".toByteArray(), initFp.toByteArray(), "|".toByteArray(), respFp.toByteArray(), "|".toByteArray(), na, nb)
            val n = ((d[0].toLong() and 0xFF) shl 24 or ((d[1].toLong() and 0xFF) shl 16) or ((d[2].toLong() and 0xFF) shl 8) or (d[3].toLong() and 0xFF)) % 1_000_000
            return "%03d %03d".format(n / 1000, n % 1000)
        }

        fun cleanName(s: String): String = s.filter { !it.isISOControl() }.trim().replace(Regex("\\s+"), " ").take(40)

        fun safeName(s: String): String {
            val n = s.substringAfterLast('/').substringAfterLast('\\').filter { !it.isISOControl() && it !in "<>:\"|?*" }.trim().trimStart('.').take(120)
            return n.ifEmpty { "file" }
        }

        fun validMid(m: String) = m.isNotEmpty() && m.length <= 32 && m.all { it.isLetterOrDigit() && it.code < 128 }
        fun isFp(s: String) = s.length == 64 && s.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }

        fun splitAddr(a: String): Pair<String, Int>? {
            val i = a.lastIndexOf(':')
            if (i <= 0) return null
            val port = a.substring(i + 1).toIntOrNull() ?: return null
            return a.substring(0, i).trim('[', ']') to port
        }

        fun joinAddr(host: String, port: Int) = if (host.contains(':')) "[$host]:$port" else "$host:$port"

        /** The paired devices saved in `dir`, without starting an engine (all shown as not online). */
        fun listPeers(dir: File): List<Peer> = try {
            Json.parseToJsonElement(File(dir, "p2p.json").readText()).jsonObject["peers"]?.jsonArray?.mapNotNull { p ->
                val po = p.jsonObject
                val fp = (po["fp"] as? JsonPrimitive)?.contentOrNull ?: return@mapNotNull null
                if (!isFp(fp)) null else Peer(fp, (po["name"] as? JsonPrimitive)?.contentOrNull ?: "", false)
            } ?: emptyList()
        } catch (e: Exception) {
            emptyList()
        }
    }

    private val rnd = SecureRandom()
    private val ctx = Tls.context(identity)
    private val client = HttpsClient(ctx)
    private val history = History(File(dir, "p2p-notes"))
    private val inbox = File(dir, "inbox")
    private val stateFile = File(dir, "p2p.json")
    private val lock = Object()
    private val work = Executors.newCachedThreadPool { r -> Thread(r, "bridge-peer-work").also { it.isDaemon = true } }
    private val timer: ScheduledExecutorService = Executors.newSingleThreadScheduledExecutor { r -> Thread(r, "bridge-peer-timer").also { it.isDaemon = true } }
    private var server: HttpsServer? = null
    val fingerprint: String get() = identity.fingerprint

    /** The port the server listens on (after [start]). */
    val port: Int get() = server?.port ?: 0

    // ------------------------------------------------------------------------------------ saved state

    private class PeerRec(var name: String, var addr: String?, val created: Double)
    private class Q(val peer: String, val kind: String, val id: String, val text: String?, val reply: JsonObject?, val ref: String?, val ts: Double)

    private val peers = LinkedHashMap<String, PeerRec>()
    private val outbox = ArrayList<Q>()
    private var nextId = 1L

    private fun now() = System.currentTimeMillis() / 1000.0
    private fun randomId() = hex(ByteArray(8).also { rnd.nextBytes(it) })

    private fun load() {
        try {
            val o = Json.parseToJsonElement(stateFile.readText()).jsonObject
            nextId = (o["next"] as? JsonPrimitive)?.longOrNull ?: 1L
            for (p in o["peers"]?.jsonArray ?: JsonArray(emptyList())) {
                val po = p.jsonObject
                val fp = str(po, "fp")
                if (isFp(fp)) peers[fp] = PeerRec(str(po, "name"), (po["addr"] as? JsonPrimitive)?.contentOrNull, (po["created"] as? JsonPrimitive)?.doubleOrNull ?: now())
            }
            for (q in o["outbox"]?.jsonArray ?: JsonArray(emptyList())) {
                val qo = q.jsonObject
                outbox.add(Q(str(qo, "peer"), str(qo, "kind"), str(qo, "id"), (qo["text"] as? JsonPrimitive)?.contentOrNull, qo["reply"] as? JsonObject, (qo["ref"] as? JsonPrimitive)?.contentOrNull, (qo["ts"] as? JsonPrimitive)?.doubleOrNull ?: now()))
            }
        } catch (e: Exception) {
            // first start, or a damaged file: begin empty
        }
    }

    /** Must be called with `lock` held. */
    private fun save() {
        try {
            val o = buildJsonObject {
                put("v", 1)
                put("next", nextId)
                put("peers", JsonArray(peers.map { (fp, r) ->
                    buildJsonObject {
                        put("fp", fp); put("name", r.name); r.addr?.let { put("addr", it) }; put("created", r.created)
                    }
                }))
                put("outbox", JsonArray(outbox.map { q ->
                    buildJsonObject {
                        put("peer", q.peer); put("kind", q.kind); put("id", q.id); put("ts", q.ts)
                        q.text?.let { put("text", it) }; q.ref?.let { put("ref", it) }; q.reply?.let { put("reply", it) }
                    }
                }))
            }
            dir.mkdirs()
            val tmp = File(stateFile.path + ".tmp")
            tmp.writeText(o.toString())
            if (!tmp.renameTo(stateFile)) {
                stateFile.delete()
                tmp.renameTo(stateFile)
            }
        } catch (e: Exception) {
            // keep running with what is in memory
        }
    }

    private fun str(o: JsonObject, k: String) = (o[k] as? JsonPrimitive)?.contentOrNull ?: ""

    // ------------------------------------------------------------------------------------ start / stop

    fun start(): Int {
        synchronized(lock) { load() }
        inbox.mkdirs()
        // half-received files of an earlier run cannot be resumed: the sender is told "cancelled" when it asks
        inbox.listFiles()?.filter { it.name.endsWith(".part") }?.forEach { it.delete() }
        val s = HttpsServer(ctx, wantedPort, bind, ::route)
        s.start()
        server = s
        timer.scheduleWithFixedDelay({ runCatching { flush() } }, 4, 4, TimeUnit.SECONDS)
        timer.scheduleWithFixedDelay({ runCatching { probe() } }, 3, 20, TimeUnit.SECONDS)
        return s.port
    }

    fun stop() {
        server?.stop()
        server = null
        timer.shutdownNow()
        work.shutdownNow()
        synchronized(lock) {
            sending.values.forEach { it.cancelled.set(true); it.abort.abort() }
            lock.notifyAll()
        }
    }

    private fun changed() = try { onChange() } catch (e: Exception) { }

    // ------------------------------------------------------------------------------------ what the app shows

    fun peers(): List<Peer> = synchronized(lock) {
        peers.map { (fp, r) -> Peer(fp, r.name, (lastOk[fp] ?: 0L) > System.currentTimeMillis() - ONLINE_MS) }
    }

    fun peerName(fp: String): String? = synchronized(lock) { peers[fp]?.name }

    fun isPeer(fp: String) = synchronized(lock) { peers.containsKey(fp) }

    /** The conversation with one paired device: what was delivered, then the texts that wait to be sent. */
    fun notes(fp: String): List<Note> {
        val waiting = synchronized(lock) { outbox.filter { it.peer == fp && it.kind == "text" }.map { pendingNote(it) } }
        return history.notes(fp) + waiting
    }

    private fun pendingNote(q: Q) = Note(
        id = PENDING_BASE + (q.ts * 1000).toLong(), mid = q.id, ts = q.ts, src = name(), kind = "text", text = q.text, pending = true,
        reply = q.reply?.let { Reply(str(it, "mid"), if ((it["mine"] as? JsonPrimitive)?.booleanOrNull == true) "x" else "pc", str(it, "preview")) },
    )

    /** Texts and files that wait for this device to come back. */
    fun waiting(fp: String): Int = synchronized(lock) { outbox.count { it.peer == fp } }

    fun transfers(): List<Xfer> = synchronized(lock) { xfers.values.sortedBy { it.id } }

    fun pairRequests(): List<InPair> = synchronized(lock) {
        purgePairs()
        incoming.filter { it.value.state == PIn.Pending || it.value.state == PIn.Accepted }.map { (id, p) -> InPair(id, p.name, p.code, p.state == PIn.Accepted) }
    }

    fun outPairs(): List<OutPair> = synchronized(lock) { outgoing.map { (id, o) -> o.snapshot(id) } }

    // ------------------------------------------------------------------------------------ talking to a paired device

    private val lastOk = ConcurrentHashMap<String, Long>()
    private val lastGood = ConcurrentHashMap<String, Pair<String, Int>>()

    private fun touch(fp: String) {
        lastOk[fp] = System.currentTimeMillis()
    }

    private fun candidates(fp: String): List<Pair<String, Int>> {
        val stored = synchronized(lock) { peers[fp]?.addr }?.let { splitAddr(it) }
        return (finder.addresses(fp) + listOfNotNull(stored)).distinct()
    }

    private fun refusedRemoved(fp: String): PeerFailure {
        removePeer(fp, tell = false)
        return PeerFailure("The other device no longer has this phone paired, so the pairing was removed here too.")
    }

    /** Find a working address for a paired device (ping, pinned to its certificate). */
    private fun connect(fp: String): Pair<String, Int> {
        if (!isPeer(fp)) throw PeerFailure("This device is not paired.")
        for ((h, p) in candidates(fp)) {
            val r = try {
                client.call(h, p, "GET", "/peer/ping", pin = fp, connectMs = 3000, readMs = 4000)
            } catch (e: PinMismatch) {
                throw PeerFailure("This device's identity has changed since it was paired. Unpair it and pair again only if you trust the change.")
            } catch (e: IOException) {
                continue
            }
            if (r.status == 403) throw refusedRemoved(fp)
            if (!r.ok) continue
            (r.json["name"] as? JsonPrimitive)?.contentOrNull?.let { updateName(fp, it) }
            touch(fp)
            lastGood[fp] = h to p
            remember(fp, joinAddr(h, p))
            return h to p
        }
        throw Offline()
    }

    private fun remember(fp: String, addr: String) = synchronized(lock) {
        val r = peers[fp]
        if (r != null && r.addr != addr) {
            r.addr = addr
            save()
        }
    }

    private fun updateName(fp: String, n: String) {
        val c = cleanName(n)
        if (c.isEmpty()) return
        val change = synchronized(lock) {
            val r = peers[fp]
            if (r != null && r.name != c) { r.name = c; save(); true } else false
        }
        if (change) changed()
    }

    /** One request to a paired device. Tries the address that worked last, then looks again. Throws [Offline] if nobody answers. */
    private fun request(fp: String, method: String, path: String, body: ByteArray? = null, readMs: Int = 15000, abort: Abort? = null): Response {
        lastGood[fp]?.let { (h, p) ->
            try {
                return checked(fp, client.call(h, p, method, path, body, pin = fp, readMs = readMs, abort = abort))
            } catch (e: PinMismatch) {
                throw PeerFailure("This device's identity has changed since it was paired. Unpair it and pair again only if you trust the change.")
            } catch (e: IOException) {
                if (abort?.aborted == true) throw e
                lastGood.remove(fp)
            }
        }
        val (h, p) = connect(fp)
        try {
            return checked(fp, client.call(h, p, method, path, body, pin = fp, readMs = readMs, abort = abort))
        } catch (e: PinMismatch) {
            throw PeerFailure("This device's identity has changed since it was paired. Unpair it and pair again only if you trust the change.")
        } catch (e: IOException) {
            if (abort?.aborted == true) throw e
            lastGood.remove(fp)
            throw Offline()
        }
    }

    private fun checked(fp: String, r: Response): Response {
        if (r.status == 403 && r.error() == "not_paired") throw refusedRemoved(fp)
        touch(fp)
        return r
    }

    private fun explain(code: String) = when (code) {
        "text_too_long" -> "That text is too long."
        "empty_text" -> "Nothing to send."
        "file_too_large" -> "The other device does not accept files that large."
        "too_many_transfers" -> "The other device already has too many files waiting."
        "id_in_use", "bad_id" -> "The other device refused this (id)."
        else -> "The other device refused it ($code)."
    }

    // ------------------------------------------------------------------------------------ messages

    private fun addNote(fp: String, n: Note): Note {
        val note = synchronized(lock) {
            val withId = n.copy(id = nextId++)
            save()
            withId
        }
        history.add(fp, note)
        changed()
        return note
    }

    /**
     * Send a text. If the device cannot be reached, it is kept (and shown as waiting) and goes out as soon as the
     * device is back. Throws [PeerFailure] when the device refuses it.
     */
    fun sendText(fp: String, text: String, replyToMid: String? = null): Note? {
        if (text.isBlank()) throw PeerFailure("Nothing to send.")
        if (text.codePointCount(0, text.length) > MAX_TEXT_CHARS) throw PeerFailure(explain("text_too_long"))
        if (!isPeer(fp)) throw PeerFailure("This device is not paired.")
        val mid = randomId()
        val orig = replyToMid?.let { m -> history.notes(fp).firstOrNull { it.mid == m } }
        val reply = orig?.let { Reply(it.mid, it.src, (it.text ?: it.file?.name ?: "").take(160)) }
        val replyJson = orig?.let { buildJsonObject { put("mid", it.mid); put("mine", !it.fromPc); put("preview", reply!!.preview) } }
        val stamp = now()
        // something is already waiting for this device: stay behind it so the order is kept
        if (waiting(fp) > 0) {
            enqueueText(fp, mid, text, replyJson, stamp)
            return null
        }
        val sent = try {
            val r = request(fp, "POST", "/peer/messages", buildJsonObject { put("text", text); put("from", name()); put("mid", mid); put("reply", replyJson ?: JsonNull) }.toString().toByteArray())
            if (!r.ok) throw PeerFailure(explain(r.error()))
            true
        } catch (e: Offline) {
            false
        }
        if (!sent) {
            enqueueText(fp, mid, text, replyJson, stamp)
            return null
        }
        return addNote(fp, Note(0, mid, stamp, name(), "text", text, null, reply))
    }

    private fun enqueueText(fp: String, mid: String, text: String, reply: JsonObject?, ts: Double) {
        synchronized(lock) {
            outbox.add(Q(fp, "text", mid, text, reply, null, ts))
            save()
        }
        changed()
    }

    /** Delete a message here and ask the other device to drop it too (best effort: if it is off, its copy stays). */
    fun deleteNote(fp: String, note: Note) {
        if (note.pending) {
            synchronized(lock) { outbox.removeAll { it.id == note.mid && it.peer == fp }; save() }
            changed()
            return
        }
        history.remove(fp, note.id)
        changed()
        if (note.mid.isNotEmpty()) {
            work.execute {
                try {
                    request(fp, "POST", "/peer/notes/delete", buildJsonObject { put("mid", note.mid) }.toString().toByteArray(), readMs = 8000)
                } catch (e: Exception) {
                }
            }
        }
    }

    /** "Clear chat" on this phone: the texts are deleted here; the other device is not asked to do anything. */
    fun clearChat(fp: String) {
        val top = history.notes(fp).maxOfOrNull { it.id } ?: return
        history.clearTexts(fp, top)
        changed()
    }

    // ------------------------------------------------------------------------------------ the waiting queue

    private val flushing = AtomicBoolean(false)

    /** Ask every paired device whether it is there, so the list can show who is online. */
    private fun probe() {
        for (p in peers()) {
            if (waiting(p.fp) > 0) continue // the queue's own round looks after these
            work.execute { try { connect(p.fp) } catch (e: Exception) { } }
        }
    }

    /** Look right now whether one paired device is there (the app calls this when the user switches to it). */
    fun check(fp: String) {
        work.execute {
            try { connect(fp) } catch (e: Exception) { }
            changed()
        }
    }

    /** Send what is waiting, oldest first, to the devices that can be reached now. Runs every few seconds. */
    fun flush() {
        if (!flushing.compareAndSet(false, true)) return
        try {
            val fps = synchronized(lock) { outbox.map { it.peer }.distinct() }
            for (fp in fps) {
                if (!isPeer(fp)) {
                    synchronized(lock) { outbox.removeAll { it.peer == fp }; save() }
                    continue
                }
                try {
                    connect(fp)
                } catch (e: Offline) {
                    continue
                } catch (e: PeerFailure) {
                    continue
                }
                while (true) {
                    val q = synchronized(lock) { outbox.firstOrNull { it.peer == fp } } ?: break
                    if (q.kind == "text") {
                        val r = try {
                            request(fp, "POST", "/peer/messages", buildJsonObject { put("text", q.text ?: ""); put("from", name()); put("mid", q.id); put("reply", q.reply ?: JsonNull) }.toString().toByteArray())
                        } catch (e: Exception) {
                            break
                        }
                        synchronized(lock) { outbox.remove(q); save() }
                        if (r.ok) {
                            val reply = q.reply?.let { Reply(str(it, "mid"), if ((it["mine"] as? JsonPrimitive)?.booleanOrNull == true) name() else "pc", str(it, "preview")) }
                            addNote(fp, Note(0, q.id, q.ts, name(), "text", q.text, null, reply))
                        } else {
                            changed() // refused for good: dropped instead of asking again and again
                        }
                    } else {
                        synchronized(lock) { outbox.remove(q); save() }
                        xfers.remove(q.id)
                        val src = q.ref?.let(openRef)
                        if (src == null) {
                            changed()
                        } else {
                            work.execute { runCatching { sendFileNow(fp, src, q.ref, q.id) } }
                        }
                    }
                }
            }
        } finally {
            flushing.set(false)
        }
    }

    // ------------------------------------------------------------------------------------ files: sending

    private class Sending(val abort: Abort = Abort(), val cancelled: AtomicBoolean = AtomicBoolean(false))

    private val sending = ConcurrentHashMap<String, Sending>()
    private val xfers = ConcurrentHashMap<String, Xfer>()

    private fun setX(x: Xfer) {
        xfers[x.id] = x
        changed()
    }

    /**
     * Send a file: checksum first, then ask the other user, then chunks that resume after a dropped connection. If the
     * device is away the file waits (`ref` is how to open it again later). Returns the transfer id.
     */
    fun sendFile(fp: String, source: FileSource, ref: String?): String {
        if (!isPeer(fp)) throw PeerFailure("This device is not paired.")
        val tid = randomId()
        val peerName = peerName(fp) ?: ""
        setX(Xfer(tid, fp, peerName, safeName(source.name), source.size, 0, false, XState.Active))
        work.execute { runCatching { sendFileNow(fp, source, ref, tid) } }
        return tid
    }

    private fun sendFileNow(fp: String, source: FileSource, ref: String?, tid: String) {
        val handle = Sending()
        sending[tid] = handle
        val name = safeName(source.name)
        val size = source.size
        val peerName = peerName(fp) ?: ""
        fun x(state: XState, done: Long = 0, error: String? = null) = setX(Xfer(tid, fp, peerName, name, size, done, false, state, error))
        try {
            val sha = try {
                val d = MessageDigest.getInstance("SHA-256")
                source.open(0).use { s ->
                    val buf = ByteArray(64 * 1024)
                    while (true) {
                        if (handle.cancelled.get()) throw CancelledByUs()
                        val n = s.read(buf)
                        if (n < 0) break
                        d.update(buf, 0, n)
                    }
                }
                hex(d.digest())
            } catch (e: CancelledByUs) {
                throw e
            } catch (e: Exception) {
                x(XState.Failed, error = "Cannot read the file.")
                return
            }
            val create = try {
                request(fp, "POST", "/peer/files", buildJsonObject { put("id", tid); put("name", name); put("size", size); put("sha256", sha) }.toString().toByteArray(), abort = handle.abort)
            } catch (e: Offline) {
                if (ref == null) {
                    x(XState.Failed, error = "The other device is not reachable.")
                } else {
                    synchronized(lock) { outbox.add(Q(fp, "file", tid, null, null, ref, now())); save() }
                    x(XState.Queued)
                }
                return
            }
            if (!create.ok) {
                x(XState.Failed, error = explain(create.error()))
                return
            }
            val remoteId = (create.json["id"] as? JsonPrimitive)?.contentOrNull ?: tid
            var state = (create.json["state"] as? JsonPrimitive)?.contentOrNull ?: "receiving"
            val statusPath = "/peer/files/$remoteId"

            if (state == "pending") {
                x(XState.Offered)
                var fails = 0
                while (true) {
                    Thread.sleep(700)
                    if (handle.cancelled.get()) throw CancelledByUs()
                    val r = try {
                        request(fp, "GET", statusPath, readMs = 8000, abort = handle.abort)
                    } catch (e: Exception) {
                        if (handle.cancelled.get()) throw CancelledByUs()
                        if (++fails > 20) throw e
                        continue
                    }
                    fails = 0
                    if (r.status == 404) {
                        x(XState.Cancelled, error = "Cancelled by $peerName")
                        return
                    }
                    when ((r.json["state"] as? JsonPrimitive)?.contentOrNull) {
                        "pending" -> {}
                        "declined" -> { x(XState.Declined, error = "Declined by $peerName"); return }
                        "cancelled" -> { x(XState.Cancelled, error = "Cancelled by $peerName"); return }
                        else -> { state = "receiving"; break }
                    }
                }
            }
            x(XState.Active)
            if (state != "done") {
                var offset = 0L
                var fails = 0
                val buf = ByteArray(UPLOAD_CHUNK)
                while (offset < size) {
                    if (handle.cancelled.get()) throw CancelledByUs()
                    val want = minOf(size - offset, UPLOAD_CHUNK.toLong()).toInt()
                    try {
                        source.open(offset).use { s ->
                            var n = 0
                            while (n < want) {
                                val r = s.read(buf, n, want - n)
                                if (r < 0) throw IOException("The file got shorter.")
                                n += r
                            }
                        }
                    } catch (e: IOException) {
                        x(XState.Failed, offset, "Cannot read the file.")
                        return
                    }
                    val r = try {
                        request(fp, "PUT", "$statusPath?offset=$offset", buf.copyOf(want), readMs = 120_000, abort = handle.abort)
                    } catch (e: Exception) {
                        if (handle.cancelled.get()) throw CancelledByUs()
                        if (++fails > MAX_RETRIES) {
                            x(XState.Failed, offset, if (e is PeerFailure) e.message else "The connection to $peerName was lost.")
                            return
                        }
                        Thread.sleep(500L * fails)
                        // ask how much arrived, then carry on from there
                        runCatching { request(fp, "GET", statusPath, readMs = 10_000) }.getOrNull()?.let { st ->
                            (st.json["received"] as? JsonPrimitive)?.longOrNull?.let { offset = minOf(it, size) }
                        }
                        continue
                    }
                    if (r.ok) {
                        offset += want
                        fails = 0
                        x(XState.Active, offset)
                        continue
                    }
                    val code = r.error()
                    if (r.status == 404 || code == "cancelled") {
                        x(XState.Cancelled, offset, "Cancelled by $peerName")
                        return
                    }
                    if (r.status == 409) {
                        val rec = (r.json["received"] as? JsonPrimitive)?.longOrNull
                        if (rec != null) {
                            offset = minOf(rec, size)
                            continue
                        }
                    }
                    x(XState.Failed, offset, if (code == "checksum_mismatch") "The file arrived damaged. Try again." else explain(code))
                    return
                }
            }
            x(XState.Done, size)
            addNote(fp, Note(0, tid, now(), name(), "file", null, FileRef(tid, name, size, sha, "to_pc")))
        } catch (e: CancelledByUs) {
            runCatching { request(fp, "DELETE", "/peer/files/$tid", readMs = 4000) }
            x(XState.Cancelled, error = "Cancelled")
        } catch (e: PeerFailure) {
            x(XState.Failed, error = e.message)
        } catch (e: Exception) {
            x(XState.Failed, error = "The connection to $peerName was lost.")
        } finally {
            sending.remove(tid)
        }
    }

    private class CancelledByUs : Exception()

    /** Stop sending a file (it may still be waiting to be accepted, or for the other device to come back). */
    fun cancelTransfer(id: String) {
        val s = sending[id]
        if (s != null) {
            s.cancelled.set(true)
            s.abort.abort()
            return
        }
        val dropped = synchronized(lock) { val r = outbox.removeAll { it.id == id && it.kind == "file" }; if (r) save(); r }
        if (dropped) {
            xfers[id]?.let { setX(it.copy(state = XState.Cancelled, error = "Cancelled")) }
            return
        }
        val inc = synchronized(lock) { incomingFiles[id] }
        if (inc != null) stopIncoming(id, InState.Cancelled, "Cancelled")
    }

    /** Remove a finished, cancelled or declined card from the list here and on the other device. */
    fun dismissTransfer(id: String) {
        val x = xfers.remove(id) ?: return
        changed()
        work.execute {
            try {
                request(x.peerFp, "POST", "/peer/transfers/$id/dismiss", "{}".toByteArray(), readMs = 8000)
            } catch (e: Exception) {
            }
        }
    }

    // ------------------------------------------------------------------------------------ files: receiving

    private enum class InState { Pending, Receiving, Finalizing, Done, Declined, Cancelled }

    private class In(
        val id: String, val fp: String, val peerName: String, val name: String, val size: Long, val sha: String?,
        val created: Long = System.currentTimeMillis(),
    ) {
        var received = 0L
        var state = InState.Pending
        var busy = false
    }

    private val incomingFiles = LinkedHashMap<String, In>()

    private fun partFile(id: String) = File(inbox, "$id.part")

    /** Where a received file is, until the app has moved it somewhere else. */
    fun inboxFile(id: String): File? = File(inbox, "$id.file").takeIf { it.isFile }

    fun discardInbox(id: String) {
        File(inbox, "$id.file").delete()
    }

    private fun incomingX(i: In, state: XState, error: String? = null) =
        setX(Xfer(i.id, i.fp, i.peerName, i.name, i.size, if (state == XState.Done) i.size else i.received, true, state, error))

    /** The user says yes. */
    fun acceptFile(id: String): Boolean {
        val i: In
        synchronized(lock) {
            i = incomingFiles[id] ?: return false
            if (i.state != InState.Pending) return false
            inbox.mkdirs()
            partFile(id).writeBytes(ByteArray(0))
            i.state = InState.Receiving
        }
        incomingX(i, XState.Active)
        if (i.size == 0L) finalizeIncoming(i)
        return true
    }

    /** The user says no. The sender finds out when it asks. */
    fun declineFile(id: String): Boolean = stopIncoming(id, InState.Declined, "Declined")

    private fun stopIncoming(id: String, to: InState, text: String): Boolean {
        val i: In
        synchronized(lock) {
            i = incomingFiles[id] ?: return false
            if (i.state != InState.Pending && i.state != InState.Receiving) return false
            i.state = to
        }
        partFile(id).delete()
        incomingX(i, if (to == InState.Declined) XState.Declined else XState.Cancelled, text)
        return true
    }

    private fun finalizeIncoming(i: In) {
        synchronized(lock) { i.state = InState.Finalizing }
        val part = partFile(i.id)
        val ok = try {
            if (i.sha == null) true else {
                val d = MessageDigest.getInstance("SHA-256")
                part.inputStream().use { s ->
                    val buf = ByteArray(64 * 1024)
                    while (true) { val n = s.read(buf); if (n < 0) break; d.update(buf, 0, n) }
                }
                hex(d.digest()) == i.sha
            }
        } catch (e: Exception) {
            false
        }
        if (!ok || !part.renameTo(File(inbox, "${i.id}.file"))) {
            part.delete()
            synchronized(lock) { i.state = InState.Cancelled }
            incomingX(i, XState.Failed, "The file arrived damaged.")
            throw Fail(422, "checksum_mismatch")
        }
        synchronized(lock) { i.state = InState.Done }
        incomingX(i, XState.Done)
        val note = addNote(i.fp, Note(0, i.id, now(), "pc", "file", null, FileRef(i.id, i.name, i.size, i.sha ?: "", "to_phone")))
        try { onFileReceived(i.fp, note, File(inbox, "${i.id}.file")) } catch (e: Exception) { }
    }

    // ------------------------------------------------------------------------------------ pairing: this phone asks

    private class OutRec(
        val host: String, val port: Int, val peerFp: String, var peerName: String, val code: String,
        var state: PairState = PairState.Waiting, var error: String? = null,
        var userConfirmed: Boolean = false, var peerAccepted: Boolean = false,
    ) {
        fun snapshot(id: String) = OutPair(id, peerName, code, state, error, peerFp, userConfirmed)
    }

    private val outgoing = LinkedHashMap<String, OutRec>()

    /** Ask the device at `host:port` to pair. Returns once the code is known; the rest happens in the background. */
    fun pairStart(host: String, port: Int): OutPair {
        val na = ByteArray(16).also { rnd.nextBytes(it) }
        val myPort = this.port
        val probe = try {
            client.call(host, port, "POST", "/peer/pair", buildJsonObject { put("name", name()); put("commit", hex(commitment(na))); put("port", myPort) }.toString().toByteArray(), pin = null, connectMs = 4000, readMs = 10_000)
        } catch (e: IOException) {
            throw PeerFailure("Could not reach that device.")
        }
        if (!probe.ok) throw PeerFailure(explainPair(probe.error()))
        val peerFp = probe.fp
        if (peerFp == fingerprint) throw PeerFailure("That is this phone.")
        val id = (probe.json["id"] as? JsonPrimitive)?.contentOrNull ?: throw PeerFailure("Unexpected answer from the other device.")
        val nb = (probe.json["nonce"] as? JsonPrimitive)?.contentOrNull?.let { unhex(it) }?.takeIf { it.size == 16 } ?: throw PeerFailure("Unexpected answer from the other device.")
        val peerName = cleanName((probe.json["name"] as? JsonPrimitive)?.contentOrNull ?: "").ifEmpty { "Unnamed device" }
        // From here on only the certificate we just saw is acceptable.
        val rv = try {
            client.call(host, port, "POST", "/peer/pair/$id/reveal", buildJsonObject { put("nonce", hex(na)) }.toString().toByteArray(), pin = peerFp, readMs = 10_000)
        } catch (e: PinMismatch) {
            throw PeerFailure("The other device changed while pairing. Nothing was paired.")
        } catch (e: IOException) {
            throw PeerFailure("Could not reach that device.")
        }
        if (!rv.ok) throw PeerFailure(explainPair(rv.error()))
        val rec = OutRec(host, port, peerFp, peerName, pairCode(fingerprint, peerFp, na, nb))
        synchronized(lock) { outgoing[id] = rec }
        changed()
        work.execute { runCatching { pairPoll(id, rec) } }
        return rec.snapshot(id)
    }

    private fun explainPair(code: String) = when (code) {
        "rate_limited", "too_many_requests" -> "That device is busy with other pairing requests. Try again in a minute."
        "that_is_me" -> "That is this phone."
        "bad_commit" -> "The other device did not accept the request. Try again."
        else -> "The other device refused it ($code)."
    }

    private fun finishOut(id: String, state: PairState, error: String?) {
        synchronized(lock) { outgoing[id]?.let { it.state = state; it.error = error }; lock.notifyAll() }
        changed()
    }

    private fun pairPoll(id: String, rec: OutRec) {
        val started = System.currentTimeMillis()
        while (true) {
            if (synchronized(lock) { !outgoing.containsKey(id) }) return // cancelled
            if (System.currentTimeMillis() - started > PAIR_TTL_MS + 10_000) {
                finishOut(id, PairState.Failed, "The other device did not answer in time.")
                return
            }
            val r = try {
                client.call(rec.host, rec.port, "GET", "/peer/pair/$id?wait=20", pin = rec.peerFp, readMs = 30_000)
            } catch (e: Exception) {
                Thread.sleep(1000)
                continue
            }
            if (r.status == 404) {
                finishOut(id, PairState.Failed, "The other device dropped the request.")
                return
            }
            when ((r.json["state"] as? JsonPrimitive)?.contentOrNull) {
                "accepted" -> {
                    val confirmed = synchronized(lock) {
                        rec.peerAccepted = true
                        if (!rec.userConfirmed) rec.state = PairState.PeerAccepted
                        rec.userConfirmed
                    }
                    if (!confirmed) changed()
                    val acceptedAt = System.currentTimeMillis()
                    // wait for our user; the other device keeps the request for a while after its user said yes
                    while (true) {
                        val st = synchronized(lock) { if (!outgoing.containsKey(id)) null else rec.userConfirmed }
                        if (st == null) return
                        if (st) { pairFinish(id, rec); return }
                        if (System.currentTimeMillis() - acceptedAt > PAIR_TTL_MS - 5000) {
                            finishOut(id, PairState.Failed, "The code was not confirmed in time.")
                            return
                        }
                        synchronized(lock) { lock.wait(100) }
                    }
                }
                "declined" -> {
                    finishOut(id, PairState.Declined, null)
                    return
                }
            }
        }
    }

    /** Both users confirmed: tell the other device, and when it has saved this phone, save it here too. */
    private fun pairFinish(id: String, rec: OutRec) {
        var err: String? = "The other device did not answer."
        for (attempt in 0 until 3) {
            if (attempt > 0) Thread.sleep(500)
            val r = try {
                client.call(rec.host, rec.port, "POST", "/peer/pair/$id/complete", "{}".toByteArray(), pin = rec.peerFp, readMs = 10_000)
            } catch (e: Exception) {
                continue
            }
            if (r.ok) { err = null; break }
            if (r.status == 404 || r.status == 409) { err = "The other device cancelled the pairing."; break }
        }
        if (err != null) {
            finishOut(id, PairState.Failed, err)
            return
        }
        synchronized(lock) {
            val created = peers[rec.peerFp]?.created ?: now()
            peers[rec.peerFp] = PeerRec(rec.peerName, joinAddr(rec.host, rec.port), created)
            save()
        }
        touch(rec.peerFp)
        finishOut(id, PairState.Done, null)
    }

    /** The user compared the codes and they are the same. */
    fun pairConfirm(id: String): Boolean = synchronized(lock) {
        val r = outgoing[id] ?: return false
        if (r.state == PairState.Done || r.state == PairState.Declined || r.state == PairState.Failed) return false
        r.userConfirmed = true
        lock.notifyAll()
        true
    }

    fun pairCancel(id: String) {
        val rec = synchronized(lock) { outgoing.remove(id).also { lock.notifyAll() } } ?: return
        changed()
        if (rec.state == PairState.Done) return
        work.execute { runCatching { client.call(rec.host, rec.port, "POST", "/peer/pair/$id/cancel", "{}".toByteArray(), pin = rec.peerFp, readMs = 4000) } }
    }

    /** Forget a finished or failed pairing card. */
    fun pairDismiss(id: String) {
        val rec = synchronized(lock) { outgoing[id] }
        if (rec != null && (rec.state == PairState.Done || rec.state == PairState.Failed || rec.state == PairState.Declined)) {
            synchronized(lock) { outgoing.remove(id) }
            changed()
        } else {
            pairCancel(id)
        }
    }

    // ------------------------------------------------------------------------------------ pairing: another device asks

    private enum class PIn { AwaitingReveal, Pending, Accepted, Done, Declined }

    private class PendingIn(
        val fp: String, val name: String, val commit: ByteArray, val nb: ByteArray, var code: String = "",
        var state: PIn = PIn.AwaitingReveal, var created: Long = System.currentTimeMillis(), val addr: String?,
    )

    private val incoming = LinkedHashMap<String, PendingIn>()
    private val pairRate = ArrayDeque<Long>()

    private fun purgePairs() {
        val now = System.currentTimeMillis()
        val gone = incoming.filter { now - it.value.created > PAIR_TTL_MS && it.value.state != PIn.Done }.keys.toList()
        if (gone.isNotEmpty()) { gone.forEach { incoming.remove(it) }; lock.notifyAll() }
    }

    /** The user of this phone compared the codes and accepts. The asking device is saved once its user confirmed too. */
    fun pairAccept(id: String): Boolean {
        val ok = synchronized(lock) {
            purgePairs()
            val p = incoming[id]
            if (p != null && p.state == PIn.Pending) {
                p.state = PIn.Accepted
                p.created = System.currentTimeMillis() // the asking user gets the full time to confirm
                lock.notifyAll()
                true
            } else false
        }
        if (ok) changed()
        return ok
    }

    fun pairDecline(id: String): Boolean {
        val ok = synchronized(lock) {
            val p = incoming[id]
            if (p != null && (p.state == PIn.Pending || p.state == PIn.AwaitingReveal || p.state == PIn.Accepted)) {
                p.state = PIn.Declined
                lock.notifyAll()
                true
            } else false
        }
        if (ok) changed()
        return ok
    }

    private fun pairRequest(fp: String, req: Request): Answer {
        if (fp == fingerprint) throw Fail(400, "that_is_me")
        val b = req.json()
        val commit = unhex(str(b, "commit"))?.takeIf { it.size == 32 } ?: throw Fail(400, "bad_commit")
        val port = (b["port"] as? JsonPrimitive)?.longOrNull?.takeIf { it in 1..65535 }?.toInt()
        val addr = port?.let { joinAddr(req.remote.substringBefore('%'), it) }
        val nm = cleanName(str(b, "name")).ifEmpty { "Unnamed device" }
        val id = randomId()
        val nb = ByteArray(16).also { rnd.nextBytes(it) }
        synchronized(lock) {
            val t = System.currentTimeMillis()
            while (pairRate.isNotEmpty() && t - pairRate.first() > 60_000) pairRate.removeFirst()
            if (pairRate.size >= 10) throw Fail(429, "rate_limited")
            pairRate.addLast(t)
            purgePairs()
            incoming.entries.removeAll { it.value.fp == fp } // one open request per device
            if (incoming.size >= MAX_PENDING_PAIRS) throw Fail(429, "too_many_requests")
            incoming[id] = PendingIn(fp, nm, commit, nb, addr = addr)
        }
        return Answer(200, buildJsonObject { put("id", id); put("nonce", hex(nb)); put("name", name()) })
    }

    private fun pairReveal(fp: String, id: String, req: Request): Answer {
        val na = unhex(str(req.json(), "nonce"))?.takeIf { it.size == 16 } ?: throw Fail(400, "bad_nonce")
        synchronized(lock) {
            purgePairs()
            val p = incoming[id]?.takeIf { it.fp == fp } ?: throw Fail(404, "not_found")
            if (p.state != PIn.AwaitingReveal) throw Fail(409, "already_revealed")
            if (!java.security.MessageDigest.isEqual(commitment(na), p.commit)) {
                incoming.remove(id)
                throw Fail(403, "bad_commit")
            }
            p.code = pairCode(fp, fingerprint, na, p.nb)
            p.state = PIn.Pending
            lock.notifyAll()
        }
        changed()
        return Answer(200, buildJsonObject { put("ok", true) })
    }

    private fun pairStatus(fp: String, id: String, wait: Double): Answer {
        val deadline = System.currentTimeMillis() + (wait.coerceIn(0.0, 25.0) * 1000).toLong()
        synchronized(lock) {
            while (true) {
                purgePairs()
                val p = incoming[id]?.takeIf { it.fp == fp } ?: throw Fail(404, "not_found")
                val name = when (p.state) {
                    PIn.AwaitingReveal -> "awaiting_reveal"
                    PIn.Pending -> "pending"
                    PIn.Accepted, PIn.Done -> "accepted"
                    PIn.Declined -> "declined"
                }
                val left = deadline - System.currentTimeMillis()
                if (name == "accepted" || name == "declined" || left <= 0) return Answer(200, buildJsonObject { put("state", name) })
                lock.wait(minOf(left, 1000))
            }
        }
    }

    private fun pairComplete(fp: String, id: String): Answer {
        synchronized(lock) {
            purgePairs()
            val p = incoming[id]?.takeIf { it.fp == fp } ?: throw Fail(404, "not_found")
            when (p.state) {
                PIn.Done -> return Answer(200, buildJsonObject { put("ok", true) })
                PIn.Accepted -> {}
                else -> throw Fail(409, "not_accepted")
            }
            p.state = PIn.Done
            val created = peers[fp]?.created ?: now()
            peers[fp] = PeerRec(p.name, p.addr ?: peers[fp]?.addr, created)
            save()
            lock.notifyAll()
        }
        touch(fp)
        changed()
        return Answer(200, buildJsonObject { put("ok", true) })
    }

    private fun pairCancelIn(fp: String, id: String): Answer {
        var removePeerToo = false
        synchronized(lock) {
            val p = incoming[id]
            if (p != null && p.fp == fp) {
                removePeerToo = p.state == PIn.Done
                incoming.remove(id)
                lock.notifyAll()
            }
        }
        if (removePeerToo) removePeer(fp, tell = false)
        changed()
        return Answer(200, buildJsonObject { put("ok", true) })
    }

    // ------------------------------------------------------------------------------------ unpairing

    /** Tell the device (best effort) and forget it here: its chat, its waiting messages, its transfers. */
    fun unpair(fp: String) {
        if (!isPeer(fp)) return
        val bases = candidates(fp)
        removePeer(fp, tell = false)
        work.execute {
            for ((h, p) in bases) {
                try {
                    client.call(h, p, "POST", "/peer/unpair", "{}".toByteArray(), pin = fp, connectMs = 3000, readMs = 4000)
                    return@execute
                } catch (e: Exception) {
                }
            }
        }
    }

    private fun removePeer(fp: String, tell: Boolean): Boolean {
        val existed = synchronized(lock) {
            val r = peers.remove(fp) != null
            outbox.removeAll { it.peer == fp }
            incoming.entries.removeAll { it.value.fp == fp }
            incomingFiles.entries.removeAll { it.value.fp == fp }
            if (r) save()
            lock.notifyAll()
            r
        }
        xfers.entries.removeAll { it.value.peerFp == fp }
        sending.entries.filter { xfers[it.key] == null }.forEach { it.value.cancelled.set(true); it.value.abort.abort() }
        lastGood.remove(fp)
        lastOk.remove(fp)
        history.forget(fp)
        if (existed) changed()
        if (tell) unpair(fp)
        return existed
    }

    // ------------------------------------------------------------------------------------ the server

    private fun route(req: Request): Answer {
        val fp = req.fp ?: throw Fail(401, "unauthorized")
        val p = req.path
        val m = req.method
        if (!p.startsWith("/peer/")) throw Fail(404, "not_found")
        // the pairing handshake is open to any device that shows a certificate
        if (m == "GET" && p == "/peer/hello") return Answer(200, buildJsonObject { put("name", name()); put("protocol", PROTOCOL); put("paired", isPeer(fp)) })
        if (m == "POST" && p == "/peer/pair") return pairRequest(fp, req)
        Regex("^/peer/pair/([0-9a-zA-Z]{1,32})(/reveal|/complete|/cancel)?$").matchEntire(p)?.let { mm ->
            val id = mm.groupValues[1]
            return when {
                m == "GET" && mm.groupValues[2].isEmpty() -> pairStatus(fp, id, req.query["wait"]?.toDoubleOrNull()?.takeIf { it.isFinite() } ?: 0.0)
                m == "POST" && mm.groupValues[2] == "/reveal" -> pairReveal(fp, id, req)
                m == "POST" && mm.groupValues[2] == "/complete" -> pairComplete(fp, id)
                m == "POST" && mm.groupValues[2] == "/cancel" -> pairCancelIn(fp, id)
                else -> throw Fail(404, "not_found")
            }
        }
        // everything else only for paired devices
        if (!isPeer(fp)) throw Fail(403, "not_paired")
        touch(fp)
        if (m == "GET" && p == "/peer/ping") return Answer(200, buildJsonObject { put("ok", true); put("name", name()); put("protocol", PROTOCOL) })
        if (m == "POST" && p == "/peer/unpair") {
            removePeer(fp, tell = false)
            return Answer(200, buildJsonObject { put("ok", true) })
        }
        if (m == "POST" && p == "/peer/messages") return receiveMessage(fp, req.json())
        if (m == "POST" && p == "/peer/notes/delete") {
            val mid = str(req.json(), "mid").takeIf { validMid(it) } ?: throw Fail(400, "bad_id")
            history.notes(fp).filter { it.mid == mid }.forEach { history.remove(fp, it.id) }
            changed()
            return Answer(200, buildJsonObject { put("ok", true) })
        }
        if (m == "POST" && p == "/peer/files") return createIncoming(fp, req.json())
        Regex("^/peer/files/([0-9a-f]{16})$").matchEntire(p)?.let { mm ->
            val id = mm.groupValues[1]
            return when (m) {
                "GET" -> incomingStatus(fp, id)
                "PUT" -> putIncoming(fp, id, req)
                "DELETE" -> {
                    val i = synchronized(lock) { incomingFiles[id]?.takeIf { it.fp == fp } } ?: throw Fail(404, "not_found")
                    when (i.state) {
                        InState.Finalizing -> throw Fail(409, "busy")
                        InState.Done -> throw Fail(409, "already_done")
                        InState.Declined, InState.Cancelled -> {}
                        else -> stopIncoming(id, InState.Cancelled, "Cancelled by ${i.peerName}")
                    }
                    Answer(200, buildJsonObject { put("cancelled", true) })
                }
                else -> throw Fail(404, "not_found")
            }
        }
        Regex("^/peer/transfers/([0-9a-f]{16})/dismiss$").matchEntire(p)?.let { mm ->
            if (m == "POST") {
                val id = mm.groupValues[1]
                if (xfers[id]?.peerFp == fp) {
                    xfers.remove(id)
                    synchronized(lock) { incomingFiles.remove(id) }
                    changed()
                }
                return Answer(200, buildJsonObject { put("ok", true) })
            }
        }
        throw Fail(404, "not_found")
    }

    private fun receiveMessage(fp: String, b: JsonObject): Answer {
        val text = (b["text"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: throw Fail(400, "empty_text")
        if (text.isBlank()) throw Fail(400, "empty_text")
        if (text.codePointCount(0, text.length) > MAX_TEXT_CHARS) throw Fail(400, "text_too_long")
        (b["from"] as? JsonPrimitive)?.contentOrNull?.let { updateName(fp, it) }
        val mid = when (val m = (b["mid"] as? JsonPrimitive)?.takeIf { it.isString }?.content) {
            null -> randomId()
            else -> if (validMid(m)) m else throw Fail(400, "bad_id")
        }
        // The quote the other device sent. "mine" is from its point of view, so it is the other way round here.
        val reply = (b["reply"] as? JsonObject)?.let { r ->
            val rm = str(r, "mid").takeIf { validMid(it) } ?: return@let null
            val mine = (r["mine"] as? JsonPrimitive)?.booleanOrNull ?: false
            Reply(rm, if (mine) "pc" else name(), str(r, "preview").take(160))
        }
        if (history.notes(fp).none { it.mid == mid }) addNote(fp, Note(0, mid, now(), "pc", "text", text, null, reply))
        return Answer(200, buildJsonObject { put("ok", true); put("name", name()) })
    }

    private fun createIncoming(fp: String, b: JsonObject): Answer {
        val nm = (b["name"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: throw Fail(400, "bad_name")
        val size = (b["size"] as? JsonPrimitive)?.longOrNull?.takeIf { it >= 0 } ?: throw Fail(400, "bad_size")
        val sha = when (val v = b["sha256"]) {
            null, is JsonNull -> null
            is JsonPrimitive -> v.contentOrNull?.takeIf { isFp(it) }?.lowercase() ?: throw Fail(400, "bad_sha256")
            else -> throw Fail(400, "bad_sha256")
        }
        val id = when (val v = b["id"]) {
            null, is JsonNull -> randomId()
            is JsonPrimitive -> v.contentOrNull?.takeIf { it.length == 16 && it.all { c -> c in '0'..'9' || c in 'a'..'f' } } ?: throw Fail(400, "bad_id")
            else -> throw Fail(400, "bad_id")
        }
        if (size > maxFileSize) throw Fail(413, "file_too_large")
        val i: In
        synchronized(lock) {
            val t = System.currentTimeMillis()
            incomingFiles.entries.removeAll { it.value.state == InState.Pending && t - it.value.created > OFFER_TTL_MS }
            if (incomingFiles.containsKey(id)) throw Fail(409, "id_in_use")
            val open = incomingFiles.values.count { it.fp == fp && (it.state == InState.Pending || it.state == InState.Receiving) }
            if (open >= MAX_OPEN_PER_PEER) throw Fail(429, "too_many_transfers")
            i = In(id, fp, peers[fp]?.name ?: "", safeName(nm), size, sha)
            incomingFiles[id] = i
        }
        incomingX(i, XState.Offered)
        if (autoAccept()) acceptFile(id)
        val state = synchronized(lock) { when (i.state) { InState.Pending -> "pending"; InState.Done -> "done"; else -> "receiving" } }
        return Answer(200, buildJsonObject { put("id", id); put("name", i.name); put("offset", 0); put("done", state == "done"); put("state", state) })
    }

    private fun incomingStatus(fp: String, id: String): Answer {
        val i = synchronized(lock) { incomingFiles[id]?.takeIf { it.fp == fp } } ?: throw Fail(404, "not_found")
        val state = when (i.state) {
            InState.Pending -> "pending"; InState.Receiving -> "receiving"; InState.Finalizing -> "finalizing"
            InState.Done -> "done"; InState.Declined -> "declined"; InState.Cancelled -> "cancelled"
        }
        return Answer(200, buildJsonObject { put("id", i.id); put("name", i.name); put("size", i.size); put("received", i.received); put("state", state); put("dir", "to_pc") })
    }

    private fun putIncoming(fp: String, id: String, req: Request): Answer {
        val offset = req.query["offset"]?.let { it.toLongOrNull() ?: throw Fail(400, "bad_query") } ?: 0L
        val i: In
        synchronized(lock) {
            i = incomingFiles[id]?.takeIf { it.fp == fp } ?: throw Fail(404, "not_found")
            when (i.state) {
                InState.Receiving -> {}
                InState.Pending -> throw Fail(409, "not_accepted")
                InState.Declined -> throw Fail(409, "declined")
                InState.Cancelled -> throw Fail(409, "cancelled")
                else -> throw Fail(409, "not_receiving")
            }
            if (i.busy) throw Fail(409, "busy")
            if (i.received != offset) throw Fail(409, "offset_mismatch", mapOf("received" to JsonPrimitive(i.received)))
            i.busy = true
        }
        try {
            if (offset + req.contentLength > i.size) throw Fail(413, "too_much_data")
            RandomAccessFile(partFile(id), "rw").use { f ->
                f.setLength(offset)
                f.seek(offset)
                val buf = ByteArray(64 * 1024)
                var left = req.contentLength
                var got = offset
                var lastEmit = 0L
                while (left > 0) {
                    if (synchronized(lock) { i.state } != InState.Receiving) throw Fail(409, "cancelled")
                    val n = req.body.read(buf, 0, minOf(buf.size.toLong(), left).toInt())
                    if (n < 0) break
                    f.write(buf, 0, n)
                    got += n
                    left -= n
                    synchronized(lock) { i.received = got }
                    val t = System.currentTimeMillis()
                    if (t - lastEmit > 250) { lastEmit = t; incomingX(i, XState.Active) }
                }
            }
            incomingX(i, XState.Active)
            if (synchronized(lock) { i.received } >= i.size) finalizeIncoming(i)
        } finally {
            synchronized(lock) { i.busy = false }
        }
        val done = synchronized(lock) { i.state == InState.Done }
        return Answer(200, buildJsonObject { put("received", synchronized(lock) { i.received }); put("done", done) })
    }
}
