package dev.bridge.phone

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Uri
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.provider.MediaStore
import android.provider.OpenableColumns
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import dev.bridge.phone.p2p.FileOnDisk
import dev.bridge.phone.p2p.Finder
import dev.bridge.phone.p2p.FileSource
import dev.bridge.phone.p2p.Identity
import dev.bridge.phone.p2p.InPair
import dev.bridge.phone.p2p.OutPair
import dev.bridge.phone.p2p.PairState
import dev.bridge.phone.p2p.Peer
import dev.bridge.phone.p2p.PeerEngine
import dev.bridge.phone.p2p.PeerFailure
import dev.bridge.phone.p2p.XState
import dev.bridge.phone.p2p.Xfer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.InputStream
import java.net.Inet4Address
import java.net.NetworkInterface
import java.util.UUID

/** A phone on the same Wi-Fi that is open in Bridge and visible. */
data class NearbyPhone(val fp: String, val name: String, val host: String, val port: Int)

/**
 * Phone to phone. This holds the [PeerEngine] (the protocol, the same one two PCs speak) and turns what it knows
 * into the flows the screens show. The engine runs while the app is on screen, and also while the app is closed if the
 * user turned on "Receive while app is closed" (a small notification keeps it alive then).
 */
object Phones {
    const val DEFAULT_PORT = 8445

    private lateinit var app: Application
    private lateinit var store: Store
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private var engine: PeerEngine? = null
    private var nsd: PhoneNsd? = null
    private var screenOn = false
    private val nearbyMap = HashMap<String, NearbyPhone>()
    private val copies = HashMap<String, File>()
    private val copying = ArrayList<Transfer>()

    private val _peers = MutableStateFlow<List<Peer>>(emptyList())
    val peers: StateFlow<List<Peer>> = _peers.asStateFlow()

    /** The phone whose chat is shown, or null while a PC's chat is shown. */
    private val _active = MutableStateFlow<Peer?>(null)
    val active: StateFlow<Peer?> = _active.asStateFlow()

    private val _notes = MutableStateFlow<List<Note>>(emptyList())
    val notes: StateFlow<List<Note>> = _notes.asStateFlow()

    private val _transfers = MutableStateFlow<List<Transfer>>(emptyList())
    val transfers: StateFlow<List<Transfer>> = _transfers.asStateFlow()

    /** Files another device wants to send, waiting for this user's answer. */
    private val _offers = MutableStateFlow<List<Xfer>>(emptyList())
    val offers: StateFlow<List<Xfer>> = _offers.asStateFlow()

    /** Another phone asks to pair (and the ones this user accepted that wait for the other user). */
    private val _requests = MutableStateFlow<List<InPair>>(emptyList())
    val requests: StateFlow<List<InPair>> = _requests.asStateFlow()

    /** Pairings this phone started. */
    private val _outPairs = MutableStateFlow<List<OutPair>>(emptyList())
    val outPairs: StateFlow<List<OutPair>> = _outPairs.asStateFlow()

    private val _nearby = MutableStateFlow<List<NearbyPhone>>(emptyList())
    val nearby: StateFlow<List<NearbyPhone>> = _nearby.asStateFlow()

    private val _cleared = MutableStateFlow<Map<String, Long>>(emptyMap())
    val cleared: StateFlow<Map<String, Long>> = _cleared.asStateFlow()

    /** Whether the active phone answered recently. */
    private val _link = MutableStateFlow(Link.Connecting)
    val link: StateFlow<Link> = _link.asStateFlow()

    private val _visible = MutableStateFlow(true)
    val visibleToPhones: StateFlow<Boolean> = _visible.asStateFlow()

    private val _background = MutableStateFlow(false)
    val receiveInBackground: StateFlow<Boolean> = _background.asStateFlow()

    private val _address = MutableStateFlow<String?>(null)
    val address: StateFlow<String?> = _address.asStateFlow()

    private val _port = MutableStateFlow(DEFAULT_PORT)
    val port: StateFlow<Int> = _port.asStateFlow()

    fun init(application: Application, s: Store) {
        app = application
        store = s
        _visible.value = store.visibleToPhones
        _background.value = store.receiveInBackground
        _cleared.value = store.phoneCleared()
        // the list of paired phones is read from the engine's file even before the engine runs
        _peers.value = PeerEngine.listPeers(File(app.filesDir, "p2p"))
        _active.value = _peers.value.firstOrNull { it.fp == store.activePhone }
        File(app.filesDir, "p2p/outgoing").listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > 7L * 24 * 3600 * 1000 }?.forEach { it.delete() }
        if (store.receiveInBackground) PeerService.start(app)
    }

    // ------------------------------------------------------------------ running

    /** The app is on screen (true) or not. The engine stops with the app unless "Receive while app is closed" is on. */
    fun setScreen(on: Boolean) {
        screenOn = on
        stopJob?.cancel()
        if (on) {
            start()
        } else if (!store.receiveInBackground) {
            // a short grace period, and never while a file is on its way
            stopJob = scope.launch {
                while (true) {
                    delay(15_000)
                    if (screenOn) return@launch
                    if (engine?.transfers()?.any { it.state == XState.Active || it.state == XState.Offered } != true) {
                        stop()
                        return@launch
                    }
                }
            }
        }
    }

    private var stopJob: Job? = null

    /** Called by the background service. */
    fun startForService() = start()

    @Synchronized
    private fun start() {
        if (engine != null) return
        val dir = File(app.filesDir, "p2p")
        dir.mkdirs()
        val identity = Identity.loadOrCreate(File(dir, "identity.bin"))
        fun make(port: Int) = PeerEngine(
            dir = dir,
            identity = identity,
            name = { store.phoneName },
            finder = Finder { fp -> synchronized(nearbyMap) { nearbyMap.values.filter { it.fp == fp }.map { it.host to it.port } } },
            openRef = { ref -> sourceFor(ref) },
            autoAccept = { false },
            onChange = { refresh() },
            onFileReceived = { _, note, file -> saved(note, file) },
            wantedPort = port,
        )
        val wanted = store.p2pPort
        var e = make(wanted)
        val bound = try {
            e.start()
        } catch (ex: Exception) {
            // the usual port is taken by something else: any free one will do, the address is found again by discovery
            e = make(0)
            e.start()
        }
        engine = e
        store.p2pPort = bound
        _port.value = bound
        nsd = PhoneNsd(app, identity.fingerprint) { list ->
            synchronized(nearbyMap) { nearbyMap.clear(); list.forEach { nearbyMap[it.fp] = it } }
            refresh()
        }.also { it.start(store.phoneName, bound, store.visibleToPhones) }
        _address.value = wifiAddress()
        refresh()
        // who is online changes without the engine saying so: look again now and then
        scope.launch {
            while (engine === e) {
                delay(10_000)
                refresh()
            }
        }
    }

    @Synchronized
    private fun stop() {
        nsd?.stop()
        nsd = null
        engine?.stop()
        engine = null
        synchronized(nearbyMap) { nearbyMap.clear() }
        refresh()
    }

    /** Re-read everything that is shown. Called by the engine whenever something changed. */
    fun refresh() {
        val e = engine
        val list = e?.peers() ?: PeerEngine.listPeers(File(app.filesDir, "p2p"))
        _peers.value = list
        val current = _active.value
        if (current != null && list.none { it.fp == current.fp }) {
            _active.value = null
            store.activePhone = null
        } else if (current != null) {
            _active.value = list.firstOrNull { it.fp == current.fp }
        }
        val act = _active.value
        _notes.value = if (e != null && act != null) e.notes(act.fp) else emptyList()
        _link.value = when {
            act == null -> Link.Connecting
            act.online -> Link.Online
            else -> Link.Offline
        }
        if (e == null) {
            _transfers.value = emptyList()
            _offers.value = emptyList()
            _requests.value = emptyList()
            _outPairs.value = emptyList()
            _nearby.value = emptyList()
            return
        }
        val xs = e.transfers()
        _transfers.value = xs.map { it.toTransfer() } + synchronized(copying) { copying.toList() }
        val offers = xs.filter { it.incoming && it.state == XState.Offered }
        val fresh = offers.filter { o -> _offers.value.none { it.id == o.id } }
        _offers.value = offers
        val reqs = e.pairRequests()
        val newReq = reqs.filter { r -> !r.accepted && _requests.value.none { it.id == r.id } }
        _requests.value = reqs
        _outPairs.value = e.outPairs()
        val paired = list.map { it.fp }.toSet()
        _nearby.value = synchronized(nearbyMap) { nearbyMap.values.filter { it.fp !in paired }.sortedBy { it.name.lowercase() } }
        if (!screenOn) {
            newReq.firstOrNull()?.let { notify("${it.name} wants to pair", "Open Bridge to compare the code.") }
            fresh.firstOrNull()?.let { notify("${it.peerName} wants to send you a file", it.name) }
        }
        // copies made for files that waited for an offline phone are not needed any more
        copies.entries.removeAll { (tid, file) ->
            val st = xs.firstOrNull { it.id == tid }?.state
            val over = st == null || st == XState.Done || st == XState.Failed || st == XState.Cancelled || st == XState.Declined
            if (over) file.delete()
            over
        }
    }

    private fun Xfer.toTransfer() = Transfer(
        key = id,
        pcId = peerFp,
        name = name,
        size = size,
        done = done,
        upload = !incoming,
        state = when (state) {
            XState.Offered -> TState.Waiting
            XState.Queued -> TState.Queued
            XState.Active -> TState.Active
            XState.Done -> TState.Done
            XState.Declined, XState.Cancelled, XState.Failed -> TState.Failed
        },
        error = error ?: when (state) {
            XState.Declined -> "Declined"
            XState.Cancelled -> "Cancelled"
            else -> null
        },
    )

    // ------------------------------------------------------------------ the chat with one phone

    fun select(peer: Peer?) {
        _active.value = peer
        store.activePhone = peer?.fp
        refresh()
        // do not wait for the next round: ask that phone now, the status line updates as soon as it answers
        if (peer != null) engine?.check(peer.fp)
    }

    private fun engineOrFail(): PeerEngine = engine ?: throw PeerFailure("Bridge is starting. Try again in a moment.")

    /** For the share sheet, which closes before the answer is known. */
    fun sendTextInBackground(peer: Peer, text: String) {
        scope.launch {
            val error = sendText(peer, text, null)
            if (error != null) Bridge.tell(error) else if ((engine?.waiting(peer.fp) ?: 0) > 0) Bridge.tell("${peer.name} is not reachable. The text is sent as soon as it is back.")
        }
    }

    /** Returns an error text, or null when the text was sent or is waiting to be sent. */
    suspend fun sendText(peer: Peer, text: String, replyTo: String?): String? = withContext(Dispatchers.IO) {
        try {
            engineOrFail().sendText(peer.fp, text, replyTo)
            null
        } catch (e: PeerFailure) {
            e.message
        } catch (e: Exception) {
            "Could not send the text."
        }
    }

    suspend fun deleteNote(peer: Peer, note: Note): String? = withContext(Dispatchers.IO) {
        try {
            engineOrFail().deleteNote(peer.fp, note)
            null
        } catch (e: PeerFailure) {
            e.message
        }
    }

    /** "Clear chat": the texts are deleted from this phone; files stay in the list of files. */
    fun clearChat(peer: Peer) {
        val top = engine?.notes(peer.fp)?.filter { !it.pending }?.maxOfOrNull { it.id } ?: return
        store.setPhoneCleared(peer.fp, top)
        _cleared.value = _cleared.value + (peer.fp to top)
        engine?.clearChat(peer.fp)
    }

    fun sendFiles(uris: List<Uri>, peer: Peer) {
        val e = engine ?: return
        scope.launch {
            for (uri in uris) {
                try {
                    val source = UriSource(app, uri)
                    // a phone that is away may not be reachable for a long time: keep a private copy, the permission to
                    // read the original ends with the screen that handed it over
                    val online = e.peers().firstOrNull { it.fp == peer.fp }?.online == true
                    if (online) {
                        e.sendFile(peer.fp, source, uri.toString())
                    } else {
                        val dir = File(app.filesDir, "p2p/outgoing").also { it.mkdirs() }
                        val copy = File(dir, UUID.randomUUID().toString().take(8) + "-" + PeerEngine.safeName(source.name))
                        // shown as busy while it is copied, which also keeps the transfer notification (and the read permission) alive
                        val marker = Transfer("copy-" + copy.name, peer.fp, source.name, source.size, upload = true)
                        synchronized(copying) { copying.add(marker) }
                        refresh()
                        try {
                            source.open(0).use { input -> copy.outputStream().use { input.copyTo(it) } }
                        } finally {
                            synchronized(copying) { copying.remove(marker) }
                        }
                        val tid = e.sendFile(peer.fp, FileOnDisk(copy), "file:" + copy.path)
                        copies[tid] = copy
                    }
                } catch (ex: PeerFailure) {
                    Bridge.tell(ex.message ?: "Could not send the file.")
                } catch (ex: Exception) {
                    Bridge.tell("Could not read the file.")
                }
            }
        }
    }

    private fun sourceFor(ref: String): FileSource? = when {
        ref.startsWith("file:") -> File(ref.removePrefix("file:")).takeIf { it.isFile }?.let { FileOnDisk(it) }
        ref.startsWith("content:") -> UriSource(app, Uri.parse(ref))
        else -> null
    }

    fun cancel(key: String) {
        engine?.cancelTransfer(key)
    }

    fun dismiss(key: String) {
        engine?.dismissTransfer(key)
    }

    fun clearFinished() {
        _transfers.value.filter { !it.running }.forEach { engine?.dismissTransfer(it.key) }
    }

    fun answer(offer: Xfer, accept: Boolean) {
        scope.launch {
            val e = engine ?: return@launch
            if (accept) e.acceptFile(offer.id) else e.declineFile(offer.id)
        }
    }

    // ------------------------------------------------------------------ received files

    /** A file arrived: move it from the app's private folder to Downloads/Bridge, like files from a PC. */
    private fun saved(note: Note, file: File) {
        val f = note.file ?: return
        scope.launch {
            try {
                val resolver = app.contentResolver
                val values = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, f.name)
                    put(MediaStore.Downloads.MIME_TYPE, Bridge.mimeOf(f.name))
                    put(MediaStore.Downloads.RELATIVE_PATH, "Download/Bridge")
                    put(MediaStore.Downloads.IS_PENDING, 1)
                }
                val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: throw IllegalStateException("no uri")
                try {
                    val out = resolver.openOutputStream(uri) ?: throw IllegalStateException("no stream")
                    out.use { o -> file.inputStream().use { it.copyTo(o) } }
                    resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
                } catch (e: Exception) {
                    resolver.delete(uri, null, null)
                    throw e
                }
                Bridge.markSaved(f.id, uri.toString())
                engine?.discardInbox(f.id)
            } catch (e: Exception) {
                Bridge.tell("Could not save ${f.name} in Downloads. It is kept inside Bridge for now.")
            }
        }
    }

    // ------------------------------------------------------------------ pairing

    /** Start pairing with a phone found nearby, or at an address typed by the user. Returns an error text, or null and the pairing id. */
    suspend fun pair(host: String, port: Int): Pair<String?, String?> = withContext(Dispatchers.IO) {
        try {
            val p = engineOrFail().pairStart(host, port)
            null to p.id
        } catch (e: PeerFailure) {
            e.message to null
        } catch (e: Exception) {
            "Could not reach that phone." to null
        }
    }

    fun confirmPair(id: String) {
        engine?.pairConfirm(id)
        refresh()
    }

    fun cancelPair(id: String) {
        engine?.pairCancel(id)
        refresh()
    }

    fun dismissPair(id: String) {
        engine?.pairDismiss(id)
        refresh()
    }

    fun acceptRequest(id: String) {
        engine?.pairAccept(id)
        refresh()
    }

    fun declineRequest(id: String) {
        engine?.pairDecline(id)
        refresh()
    }

    /** A pairing this phone started finished: show the new phone. */
    fun openPeer(fp: String) {
        refresh()
        _peers.value.firstOrNull { it.fp == fp }?.let { select(it) }
    }

    fun unpair(peer: Peer) {
        engine?.unpair(peer.fp)
        if (_active.value?.fp == peer.fp) select(null)
        store.setPhoneCleared(peer.fp, 0)
        _cleared.value = _cleared.value - peer.fp
        refresh()
    }

    // ------------------------------------------------------------------ settings

    fun setVisible(on: Boolean) {
        store.visibleToPhones = on
        _visible.value = on
        nsd?.start(store.phoneName, _port.value, on)
    }

    fun setBackground(on: Boolean) {
        store.receiveInBackground = on
        _background.value = on
        if (on) PeerService.start(app) else {
            PeerService.stop(app)
            if (!screenOn) stop()
        }
    }

    /** The name was changed: tell nearby phones. */
    fun nameChanged() {
        nsd?.start(store.phoneName, _port.value, store.visibleToPhones)
    }

    private fun wifiAddress(): String? = try {
        val cm = app.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val props = cm.getLinkProperties(cm.activeNetwork)
        props?.linkAddresses?.map { it.address }?.firstOrNull { it is Inet4Address && !it.isLoopbackAddress }?.hostAddress
            ?: NetworkInterface.getNetworkInterfaces().toList().flatMap { it.inetAddresses.toList() }
                .firstOrNull { it is Inet4Address && !it.isLoopbackAddress && it.isSiteLocalAddress }?.hostAddress
    } catch (e: Exception) {
        null
    }

    fun refreshAddress() {
        _address.value = wifiAddress()
    }

    private fun notify(title: String, text: String) {
        try {
            val manager = app.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(NotificationChannel("requests", "Requests", NotificationManager.IMPORTANCE_DEFAULT))
            val open = PendingIntent.getActivity(app, 1, Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
            val n = NotificationCompat.Builder(app, "requests").setSmallIcon(R.drawable.ic_stat).setContentTitle(title).setContentText(text).setAutoCancel(true).setContentIntent(open).build()
            manager.notify(title.hashCode(), n)
        } catch (e: Exception) {
            // notifications may not be allowed
        }
    }

    // ------------------------------------------------------------------ a file from another app

    private class UriSource(private val context: Context, private val uri: Uri) : FileSource {
        private val meta: Pair<String, Long> by lazy {
            var name = "file"
            var size = -1L
            try {
                context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                    if (c.moveToFirst()) {
                        c.getString(0)?.let { name = it }
                        if (!c.isNull(1)) size = c.getLong(1)
                    }
                }
            } catch (e: Exception) {
                // keep the defaults
            }
            if (size < 0) {
                size = try {
                    context.contentResolver.openAssetFileDescriptor(uri, "r")?.use { it.length } ?: 0L
                } catch (e: Exception) {
                    0L
                }
            }
            name to maxOf(size, 0L)
        }
        override val name: String get() = meta.first
        override val size: Long get() = meta.second
        override fun open(offset: Long): InputStream {
            val input = context.contentResolver.openInputStream(uri) ?: throw java.io.IOException("Could not open the file.")
            var left = offset
            while (left > 0) {
                val n = input.skip(left)
                if (n > 0) left -= n else if (input.read() < 0) break else left--
            }
            return input
        }
    }
}

/**
 * Announces this phone as `_bridgephone._tcp` (name, certificate fingerprint, protocol version; nothing else) when it is
 * visible, and looks for other phones announcing themselves. PCs announce `_bridgepc._tcp`, so the two lists never mix.
 */
class PhoneNsd(context: Context, private val ownFp: String, private val onNearby: (List<NearbyPhone>) -> Unit) {
    private val nsd = context.applicationContext.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val found = HashMap<String, NearbyPhone>()
    private val queue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false
    private var registration: NsdManager.RegistrationListener? = null
    private var discovery: NsdManager.DiscoveryListener? = null
    private var stopped = false

    /** (Re)publish this phone, or withdraw it when not visible; start browsing once. */
    @Synchronized
    fun start(name: String, port: Int, visible: Boolean) {
        stopped = false
        unregister()
        if (visible) register(name, port)
        if (discovery == null) browse()
    }

    @Synchronized
    fun stop() {
        stopped = true
        unregister()
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        found.clear()
        queue.clear()
    }

    private fun unregister() {
        registration?.let { runCatching { nsd.unregisterService(it) } }
        registration = null
    }

    private fun register(name: String, port: Int) {
        val info = NsdServiceInfo().apply {
            serviceName = ownFp.take(32)
            serviceType = TYPE
            this.port = port
            setAttribute("n", name.take(40))
            setAttribute("fp", ownFp)
            setAttribute("v", "1")
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onRegistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {}
            override fun onUnregistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {}
            override fun onServiceRegistered(serviceInfo: NsdServiceInfo) {}
            override fun onServiceUnregistered(serviceInfo: NsdServiceInfo) {}
        }
        registration = listener
        runCatching { nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener) }
    }

    private fun browse() {
        val listener = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(service: NsdServiceInfo) {
                synchronized(this@PhoneNsd) { queue.addLast(service) }
                next()
            }

            override fun onServiceLost(service: NsdServiceInfo) {
                val changed = synchronized(this@PhoneNsd) { found.entries.removeAll { it.key == service.serviceName } }
                if (changed) publish()
            }

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}
        }
        discovery = listener
        runCatching { nsd.discoverServices(TYPE, NsdManager.PROTOCOL_DNS_SD, listener) }
    }

    // Android resolves one service at a time, so found services wait in line.
    private fun next() {
        val service = synchronized(this) {
            if (resolving || stopped) return
            val head = queue.removeFirstOrNull() ?: return
            resolving = true
            head
        }
        try {
            @Suppress("DEPRECATION")
            nsd.resolveService(service, object : NsdManager.ResolveListener {
                override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) = resolved(service, null)
                override fun onServiceResolved(info: NsdServiceInfo) = resolved(service, info)
            })
        } catch (e: Exception) {
            resolved(service, null)
        }
    }

    private fun resolved(service: NsdServiceInfo, info: NsdServiceInfo?) {
        synchronized(this) { resolving = false }
        if (info != null) {
            val fp = info.attributes["fp"]?.let { String(it) }?.lowercase()
            val name = info.attributes["n"]?.let { String(it) }
            @Suppress("DEPRECATION")
            val host = (info.host as? Inet4Address)?.hostAddress
            if (fp != null && name != null && host != null && fp != ownFp && fp.length == 64) {
                synchronized(this) { found[service.serviceName] = NearbyPhone(fp, PeerEngine.cleanName(name).ifEmpty { "Unnamed phone" }, host, info.port) }
                publish()
            }
        }
        next()
    }

    private fun publish() {
        val list = synchronized(this) { found.values.toList() }
        onNearby(list)
    }

    companion object {
        const val TYPE = "_bridgephone._tcp."
    }
}
