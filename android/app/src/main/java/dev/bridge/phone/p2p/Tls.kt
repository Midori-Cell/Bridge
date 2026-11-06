package dev.bridge.phone.p2p

import java.net.Socket
import java.security.Principal
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.cert.Certificate
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.SSLSession
import javax.net.ssl.TrustManager
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509ExtendedTrustManager

/**
 * TLS between two Bridge devices. Both sides show a certificate and neither is checked against an authority:
 * a device is trusted because its certificate fingerprint was compared by the user when pairing (or is the one
 * pinned for a paired device). The code that connects checks the fingerprint right after the handshake and
 * before anything is sent; the code that serves looks the fingerprint up before it answers.
 */
object Tls {
    val PROTOCOLS = arrayOf("TLSv1.3", "TLSv1.2")

    fun context(id: Identity): SSLContext {
        val c = SSLContext.getInstance("TLS")
        c.init(arrayOf(OwnKey(id)), arrayOf<TrustManager>(AcceptAny), SecureRandom())
        return c
    }

    fun fingerprint(c: Certificate): String = Identity.fingerprintOf(c.encoded)

    /** The fingerprint of the certificate the other side showed on this connection, or null if it showed none. */
    fun peerFingerprint(s: SSLSession): String? = try {
        s.peerCertificates.firstOrNull()?.let { fingerprint(it) }
    } catch (e: Exception) {
        null
    }

    private class OwnKey(private val id: Identity) : X509ExtendedKeyManager() {
        private val alias = "bridge"
        override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(alias)
        override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?) = alias
        override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(alias)
        override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?) = alias
        override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?) = alias
        override fun chooseEngineServerAlias(keyType: String?, issuers: Array<out Principal>?, engine: SSLEngine?) = alias
        override fun getCertificateChain(alias: String?): Array<X509Certificate> = arrayOf(id.cert)
        override fun getPrivateKey(alias: String?): PrivateKey = id.key
    }

    private object AcceptAny : X509ExtendedTrustManager() {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) {}
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) {}
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) {}
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) {}
        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
    }
}
