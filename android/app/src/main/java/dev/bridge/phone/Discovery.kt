package dev.bridge.phone

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import java.net.Inet4Address
import kotlin.coroutines.resume

/**
 * Finds a paired PC again when its address has changed (a new DHCP lease, for example).
 * Every PC announces `_bridgepc._tcp` with its certificate fingerprint; only the PC with the pinned
 * fingerprint counts, and the connection is pinned anyway.
 */
object Discovery {
    suspend fun find(context: Context, fingerprint: String, timeoutMs: Long = 8000): String? = withTimeoutOrNull(timeoutMs) {
        suspendCancellableCoroutine<String?> { cont ->
            val nsd = context.applicationContext.getSystemService(Context.NSD_SERVICE) as NsdManager
            val finder = Finder(nsd, fingerprint, cont)
            cont.invokeOnCancellation { finder.stop() }
            try {
                nsd.discoverServices("_bridgepc._tcp.", NsdManager.PROTOCOL_DNS_SD, finder)
            } catch (e: Exception) {
                if (cont.isActive) cont.resume(null)
            }
        }
    }

    private class Finder(
        private val nsd: NsdManager,
        private val fingerprint: String,
        private val cont: CancellableContinuation<String?>,
    ) : NsdManager.DiscoveryListener {
        private val queue = ArrayDeque<NsdServiceInfo>()
        private var resolving = false
        private var stopped = false

        fun stop() {
            synchronized(this) {
                if (stopped) return
                stopped = true
            }
            try {
                nsd.stopServiceDiscovery(this)
            } catch (e: Exception) {
                // not started or already stopped
            }
        }

        private fun finish(host: String?) {
            stop()
            if (cont.isActive) cont.resume(host)
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
                    override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) = resolved(null)

                    override fun onServiceResolved(info: NsdServiceInfo) = resolved(info)
                })
            } catch (e: Exception) {
                resolved(null)
            }
        }

        private fun resolved(info: NsdServiceInfo?) {
            synchronized(this) { resolving = false }
            val announced = info?.attributes?.get("fp")?.let { String(it) }
            if (info != null && announced != null && announced.equals(fingerprint, ignoreCase = true)) {
                @Suppress("DEPRECATION")
                val address = info.host
                if (address is Inet4Address) {
                    finish(address.hostAddress)
                    return
                }
            }
            next()
        }

        override fun onServiceFound(service: NsdServiceInfo) {
            synchronized(this) { queue.addLast(service) }
            next()
        }

        override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) = finish(null)
        override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
        override fun onDiscoveryStarted(serviceType: String) {}
        override fun onDiscoveryStopped(serviceType: String) {}
        override fun onServiceLost(service: NsdServiceInfo) {}
    }
}
