package dev.bridge.phone

import android.app.Application
import android.content.ActivityNotFoundException
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.SystemClock
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

sealed interface PairResult {
    /** `unverified`: paired without the QR code, so the certificate was not checked against anything yet. */
    data class Ok(val pc: Pc, val unverified: Boolean) : PairResult
    data class Failed(val message: String) : PairResult
}

/**
 * Everything the app does with a PC: pairing, the conversation, files in both directions.
 * It lives as long as the process, so a transfer goes on when the screen closes.
 */
object Bridge {
    private const val CHUNK = 4 * 1024 * 1024

    /** How long one request for news waits. The PC shows the phone as online as long as it hears from it this often. */
    private const val POLL_SECONDS = 10

    private lateinit var app: Application
    private lateinit var store: Store
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val _pcs = MutableStateFlow<List<Pc>>(emptyList())
    val pcs: StateFlow<List<Pc>> = _pcs.asStateFlow()

    private val _active = MutableStateFlow<Pc?>(null)
    val active: StateFlow<Pc?> = _active.asStateFlow()

    private val _notes = MutableStateFlow<List<Note>>(emptyList())
    val notes: StateFlow<List<Note>> = _notes.asStateFlow()

    private val _link = MutableStateFlow(Link.Connecting)
    val link: StateFlow<Link> = _link.asStateFlow()

    private val _transfers = MutableStateFlow<List<Transfer>>(emptyList())
    val transfers: StateFlow<List<Transfer>> = _transfers.asStateFlow()

    private val _saved = MutableStateFlow<Map<String, String>>(emptyMap())
    val saved: StateFlow<Map<String, String>> = _saved.asStateFlow()

    /** Short messages for the user (a toast): "unpaired", "no longer on the PC", ... */
    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val messages: SharedFlow<String> = _messages

    private val _themeMode = MutableStateFlow(ThemeMode.System)
    val themeMode: StateFlow<ThemeMode> = _themeMode.asStateFlow()

    /** A file the PC offers that nobody has answered yet. */
    data class Offer(val file: FileRef, val pc: Pc)

    private val _offers = MutableStateFlow<List<Offer>>(emptyList())
    val offers: StateFlow<List<Offer>> = _offers.asStateFlow()

    private val clients = ConcurrentHashMap<String, OkHttpClient>()
    private val running = ConcurrentHashMap<String, Job>()
    private val uploadGate = Mutex()
    private val downloadGate = Mutex()

    private var visible = false
    private var pollJob: Job? = null
    private var notesPc: String? = null
    private var myName: String? = null

    fun init(application: Application) {
        app = application
        store = Store(application)
        _saved.value = store.saved()
        _themeMode.value = store.themeMode
        reload()
    }

    private fun say(text: String) {
        _messages.tryEmit(text)
    }

    private fun reload() {
        val list = store.pcs()
        _pcs.value = list
        _active.value = list.firstOrNull { it.id == store.activeId } ?: list.firstOrNull()
    }

    private fun api(pc: Pc): Api = Api(pc, clients.getOrPut(pc.fingerprint) { pinnedClient(pc.fingerprint) })

    var phoneName: String
        get() = store.phoneName
        set(value) {
            if (value.isNotBlank()) store.phoneName = value
        }

    fun setThemeMode(mode: ThemeMode) {
        store.themeMode = mode
        _themeMode.value = mode
    }

    var askedNotifications: Boolean
        get() = store.askedNotifications
        set(value) {
            store.askedNotifications = value
        }

    // ------------------------------------------------------------------ pairing

    suspend fun pair(host: String, port: Int, code: String, fingerprint: String?): PairResult = withContext(Dispatchers.IO) {
        try {
            val (answer, seen) = Api.pair(host, port, code.trim(), store.phoneName, fingerprint)
            if (!seen.equals(answer.fingerprint, ignoreCase = true)) {
                return@withContext PairResult.Failed("The PC's answer does not match its certificate.")
            }
            val pc = Pc(name = answer.pc.ifBlank { "Your PC" }, host = host, port = port, fingerprint = answer.fingerprint.lowercase(), token = answer.token)
            store.savePcs(_pcs.value.filter { it.id != pc.id } + pc)
            store.activeId = pc.id
            reload()
            restartPoll()
            PairResult.Ok(pc, unverified = fingerprint == null)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            PairResult.Failed(
                if (e is ApiException) {
                    when (e.code) {
                        "locked" -> "Too many wrong codes. Wait a minute and show a new code on the PC."
                        "invalid" -> "That code is wrong or expired. Show a new code on the PC."
                        "too_many_devices" -> "This PC already has as many phones as it allows. Remove one on the PC first."
                        else -> explain(e)
                    }
                } else {
                    explain(e)
                },
            )
        }
    }

    /** The user answered the question "receive this file?". */
    fun answer(offer: Offer, accept: Boolean) {
        _offers.update { l -> l.filter { it.file.id != offer.file.id } }
        if (accept) download(offer.file, offer.pc)
    }

    fun select(pc: Pc) {
        store.activeId = pc.id
        reload()
        restartPoll()
    }

    /** Tell the PC, then forget it here. If the PC cannot be reached it is forgotten here anyway. */
    fun unpair(pc: Pc) {
        pollJob?.cancel()
        scope.launch {
            try {
                api(pc).unpair()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // the PC can be unpaired from its own window later
            }
            forget(pc)
        }
    }

    private fun forget(pc: Pc) {
        _offers.update { l -> l.filter { it.pc.id != pc.id } }
        val rest = _pcs.value.filter { it.id != pc.id }
        store.savePcs(rest)
        if (store.activeId == pc.id) store.activeId = rest.firstOrNull()?.id
        reload()
        if (_active.value == null) _notes.value = emptyList()
        restartPoll()
    }

    private fun updated(pc: Pc) {
        val list = _pcs.value.map { if (it.id == pc.id) pc else it }
        store.savePcs(list)
        _pcs.value = list
        if (_active.value?.id == pc.id) _active.value = pc
    }

    // ------------------------------------------------------------------ the conversation

    /** The app is on screen (true) or not (false): the feed is only followed while it is visible. */
    fun setVisible(value: Boolean) {
        visible = value
        if (value) restartPoll() else pollJob?.cancel()
    }

    private fun restartPoll() {
        pollJob?.cancel()
        pollJob = null
        val pc = _active.value ?: return
        if (!visible) return
        pollJob = scope.launch { poll(pc) }
    }

    private suspend fun poll(start: Pc) {
        var pc = start
        var api = api(pc)
        var lastId: Long? = null
        var del: Long? = null
        var fails = 0
        if (notesPc != pc.id) {
            _notes.value = emptyList()
            notesPc = pc.id
        }
        _link.value = Link.Connecting
        try {
            myName = api.ping().device.ifBlank { null }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (e is ApiException && e.status == 401) {
                unpaired(pc)
                return
            }
        }
        while (true) {
            currentCoroutineContext().ensureActive()
            val failed = try {
                val first = lastId == null
                val r = if (first) api.notes(null, null, 0) else api.notes(lastId, del, POLL_SECONDS)
                // another phone paired with the same PC writes into the same history; only this phone's part is shown
                val mine = myName
                val news = r.notes.filter { mine == null || it.src == "pc" || it.src == mine }
                val gone = r.deleted.toSet()
                _notes.update { old -> ((if (first) emptyList() else old) + news).distinctBy { it.id }.filter { it.mid !in gone }.sortedBy { it.id }.takeLast(500) }

                // files the PC sent since the last visit are offered now (the phone user decides); the older history is left alone
                val seen = store.seen(pc.id) ?: r.lastId
                val wanted = news.mapNotNull { n -> n.file?.takeIf { n.id > seen && n.kind == "file" && it.dir == "to_phone" && it.id !in _saved.value } }
                store.setSeen(pc.id, maxOf(seen, r.lastId))
                if (wanted.isNotEmpty()) _offers.update { old -> (old + wanted.map { Offer(it, pc) }).distinctBy { o -> o.file.id } }

                val before = lastId
                lastId = if (before == null) r.lastId else maxOf(before, r.lastId)
                del = r.del
                _link.value = Link.Online
                fails = 0
                false
            } catch (e: CancellationException) {
                throw e
            } catch (e: ApiException) {
                if (e.status == 401) {
                    unpaired(pc)
                    return
                }
                true
            } catch (e: Exception) {
                true
            }
            if (failed) {
                _link.value = Link.Offline
                fails++
                if (fails == 3) {
                    // maybe the PC got a new address
                    val host = Discovery.find(app, pc.fingerprint)
                    if (host != null && host != pc.host) {
                        pc = pc.copy(host = host)
                        updated(pc)
                        api = api(pc)
                        fails = 0
                        continue
                    }
                }
                delay(minOf(1000L shl minOf(fails, 4), 15_000L))
            }
        }
    }

    private fun unpaired(pc: Pc) {
        say("${pc.name} unpaired this phone. Pair it again with a new code.")
        forget(pc)
    }

    /** Returns an error text, or null when the note was sent. */
    suspend fun sendText(text: String, replyTo: String?, pc: Pc? = _active.value): String? = withContext(Dispatchers.IO) {
        if (pc == null) return@withContext "Pair a PC first."
        try {
            val note = api(pc).sendNote(text, replyTo)
            if (pc.id == _active.value?.id) _notes.update { (it + note).distinctBy { n -> n.id }.sortedBy { n -> n.id } }
            null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (e is ApiException && e.status == 401) unpaired(pc)
            explain(e)
        }
    }

    /** For the share sheet, which closes before the answer is known. */
    fun sendTextInBackground(text: String, pc: Pc) {
        scope.launch {
            sendText(text, null, pc)?.let { say(it) }
        }
    }

    suspend fun deleteNote(note: Note): String? = withContext(Dispatchers.IO) {
        val pc = _active.value ?: return@withContext "Pair a PC first."
        try {
            api(pc).deleteNote(note.mid)
            _notes.update { l -> l.filter { it.id != note.id } }
            null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            explain(e)
        }
    }

    // ------------------------------------------------------------------ sending files

    private class Pace {
        private var last = 0L
        fun due(): Boolean {
            val now = SystemClock.elapsedRealtime()
            if (now - last < 200) return false
            last = now
            return true
        }
    }

    private fun change(key: String, f: (Transfer) -> Transfer) {
        _transfers.update { list -> list.map { if (it.key == key) f(it) else it } }
    }

    fun describe(uri: Uri): Pair<String, Long> {
        var name = "file"
        var size = -1L
        try {
            app.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    c.getString(0)?.let { name = it }
                    if (!c.isNull(1)) size = c.getLong(1)
                }
            }
            if (size < 0) app.contentResolver.openAssetFileDescriptor(uri, "r")?.use { size = it.length }
        } catch (e: Exception) {
            // keep the defaults; the upload fails with a clear message if the file cannot be read
        }
        return name to maxOf(size, 0L)
    }

    /** Send these files to `pc`, one after the other. */
    fun sendFiles(uris: List<Uri>, pc: Pc) {
        for (uri in uris) {
            val (name, size) = describe(uri)
            val key = UUID.randomUUID().toString()
            _transfers.update { it + Transfer(key, pc.id, name, size, upload = true) }
            track(key, scope.launch(start = CoroutineStart.LAZY) { uploadGate.withLock { upload(key, uri, size, pc) } })
        }
    }

    /** Remember a transfer's job so it can be cancelled, and mark it cancelled if it never got to run. */
    private fun track(key: String, job: Job) {
        running[key] = job
        job.invokeOnCompletion { cause ->
            running.remove(key)
            if (cause is CancellationException) change(key) { if (it.running) it.copy(state = TState.Failed, error = "Cancelled") else it }
        }
        job.start()
    }

    private suspend fun upload(key: String, uri: Uri, size: Long, pc: Pc) {
        val api = api(pc)
        var fileId: String? = null
        try {
            val name = _transfers.value.firstOrNull { it.key == key }?.name ?: "file"
            val created = api.createFile(name, size)
            fileId = created.id
            var state = created.state
            // the PC's user decides first; nothing is sent until the PC says yes
            while (state == "pending") {
                delay(1000)
                state = api.fileStatus(created.id).state
            }
            if (state == "declined" || state == "cancelled") throw ApiException(409, state)
            change(key) { it.copy(state = TState.Active) }
            if (!created.done) sendChunks(key, uri, size, created.id, api)
            change(key) { it.copy(state = TState.Done, done = size) }
        } catch (e: CancellationException) {
            fileId?.let { id -> withContext(NonCancellable) { runCatching { api.cancelFile(id) } } }
            throw e
        } catch (e: Exception) {
            if (e is ApiException && e.status == 401) unpaired(pc)
            change(key) { it.copy(state = TState.Failed, error = explain(e)) }
        }
    }

    private suspend fun sendChunks(key: String, uri: Uri, size: Long, id: String, api: Api) {
        fun open(): InputStream = app.contentResolver.openInputStream(uri) ?: throw Problem("Could not open the file.")
        var input = open()
        try {
            var pos = 0L
            var fails = 0
            var chunk: ByteArray? = null // the piece being sent, kept in case it has to be sent again
            val pace = Pace()
            while (pos < size) {
                val data = chunk ?: ByteArray(minOf(CHUNK.toLong(), size - pos).toInt()).also {
                    readFully(input, it)
                    chunk = it
                }
                val base = pos
                try {
                    val answer = api.putChunk(id, pos, data) { sent ->
                        if (pace.due()) change(key) { it.copy(done = base + sent) }
                    }
                    pos += data.size
                    chunk = null
                    fails = 0
                    change(key) { it.copy(done = pos) }
                    if (answer.done) break
                } catch (e: CancellationException) {
                    throw e
                } catch (e: IOException) {
                    val fatal = e is ApiException && (e.status in listOf(401, 404, 413, 422, 507) || (e.status == 409 && e.code != "offset_mismatch" && e.code != "busy"))
                    if (fatal || ++fails > 6) throw e
                    delay(1000L * fails)
                    // ask the PC how far it got and go on from there
                    val received = api.fileStatus(id).received
                    when {
                        received == pos -> {}
                        received == pos + data.size -> {
                            pos = received
                            chunk = null
                        }
                        else -> {
                            input.close()
                            input = open()
                            skipFully(input, received)
                            pos = received
                            chunk = null
                        }
                    }
                }
            }
        } finally {
            input.close()
        }
    }

    private fun readFully(input: InputStream, buffer: ByteArray) {
        var off = 0
        while (off < buffer.size) {
            val n = input.read(buffer, off, buffer.size - off)
            if (n < 0) throw Problem("The file changed while it was being sent.")
            off += n
        }
    }

    private fun skipFully(input: InputStream, count: Long) {
        var left = count
        while (left > 0) {
            val n = input.skip(left)
            if (n > 0) {
                left -= n
            } else {
                if (input.read() < 0) throw Problem("The file changed while it was being sent.")
                left--
            }
        }
    }

    // ------------------------------------------------------------------ receiving files

    private fun mimeOf(name: String): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"

    /** Save a file the PC offers into Downloads/Bridge. */
    fun download(file: FileRef, pc: Pc) {
        if (_transfers.value.any { it.key == file.id && it.running }) return
        _transfers.update { l -> l.filter { it.key != file.id } + Transfer(file.id, pc.id, file.name, file.size, upload = false) }
        try {
            ContextCompat.startForegroundService(app, Intent(app, TransferService::class.java))
        } catch (e: Exception) {
            // the app is in the background: the download still runs, only without the notification
        }
        track(file.id, scope.launch(start = CoroutineStart.LAZY) { downloadGate.withLock { fetch(file, pc) } })
    }

    private suspend fun fetch(file: FileRef, pc: Pc) {
        val resolver = app.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, file.name)
            put(MediaStore.Downloads.MIME_TYPE, mimeOf(file.name))
            put(MediaStore.Downloads.RELATIVE_PATH, "Download/Bridge")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        if (uri == null) {
            change(file.id) { it.copy(state = TState.Failed, error = "Could not create the file in Downloads.") }
            return
        }
        try {
            change(file.id) { it.copy(state = TState.Active) }
            val api = api(pc)
            val digest = MessageDigest.getInstance("SHA-256")
            val pace = Pace()
            var got = 0L
            val out = resolver.openOutputStream(uri) ?: throw Problem("Could not write to Downloads.")
            out.use {
                var attempts = 0
                while (got < file.size) {
                    try {
                        api.download(file.id, got) { buffer, n ->
                            out.write(buffer, 0, n)
                            digest.update(buffer, 0, n)
                            got += n
                            if (pace.due()) change(file.id) { it.copy(done = got) }
                        }
                        break
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: IOException) {
                        if (e is ApiException || ++attempts > 5) throw e
                        delay(1000L * attempts) // the download goes on from where it stopped
                    }
                }
            }
            if (got != file.size) throw Problem("The file arrived incomplete.")
            val sum = digest.digest().joinToString("") { "%02x".format(it) }
            if (file.sha256.isNotEmpty() && !sum.equals(file.sha256, ignoreCase = true)) throw Problem("The file arrived damaged.")
            resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            val map = _saved.value + (file.id to uri.toString())
            store.saveSaved(map)
            _saved.value = map
            change(file.id) { it.copy(state = TState.Done, done = file.size) }
        } catch (e: CancellationException) {
            resolver.delete(uri, null, null)
            throw e
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            if (e is ApiException && e.status == 401) unpaired(pc)
            change(file.id) { it.copy(state = TState.Failed, error = explain(e)) }
        }
    }

    /** Open a saved file with whatever app the user has for it. */
    fun open(context: Context, file: FileRef) {
        val saved = _saved.value[file.id] ?: return
        val uri = Uri.parse(saved)
        val exists = try {
            context.contentResolver.query(uri, arrayOf(MediaStore.Downloads._ID), null, null, null)?.use { it.moveToFirst() } ?: false
        } catch (e: Exception) {
            false
        }
        if (!exists) {
            // deleted from Downloads in the meantime: offer it again
            val map = _saved.value - file.id
            store.saveSaved(map)
            _saved.value = map
            say("${file.name} is no longer in Downloads. Tap it to save it again.")
            return
        }
        val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, mimeOf(file.name)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (e: ActivityNotFoundException) {
            say("No app on this phone can open ${file.name}. It is in Downloads/Bridge.")
        }
    }

    fun cancel(key: String) {
        running[key]?.cancel()
    }

    fun dismiss(key: String) {
        _transfers.update { l -> l.filter { it.key != key || it.running } }
    }

    fun clearFinished() {
        _transfers.update { l -> l.filter { it.running } }
    }

    // ------------------------------------------------------------------ errors in words

    fun explain(e: Throwable): String {
        if (e is ApiException) {
            return when (e.code) {
                "declined" -> "The PC declined the file."
                "cancelled" -> "Cancelled on the PC."
                "hash_mismatch" -> "The file arrived damaged."
                "disk_full" -> "The PC disk is full."
                "file_too_large" -> "That file is too large."
                "too_many_transfers" -> "Too many transfers at once."
                "text_too_long" -> "That text is too long."
                "file_gone", "file_changed" -> "The file is no longer on the PC."
                else -> "The PC answered: ${e.code}."
            }
        }
        if (generateSequence(e) { it.cause }.any { it.message?.contains("not the PC that was paired") == true }) {
            return "This PC's identity has changed since it was paired. Unpair it and pair again only if you trust the change."
        }
        if (e is Problem) return e.message ?: "Something went wrong."
        return if (e is IOException) "Could not reach the PC." else (e.message ?: "Something went wrong.")
    }
}
