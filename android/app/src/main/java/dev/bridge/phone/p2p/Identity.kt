package dev.bridge.phone.p2p

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.Signature
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.security.spec.ECGenParameterSpec
import java.security.spec.PKCS8EncodedKeySpec

/**
 * Who this phone is to other devices: an ECDSA P-256 key and a self-signed certificate. The SHA-256 of the
 * certificate (the fingerprint) is the identity; other devices pin it when they pair. Nothing is signed by an
 * authority and nothing leaves the phone, the same as on the PC.
 *
 * The key is kept in a file in the app's private storage (like the PC keeps its own in its data folder). Only
 * plain Java classes are used, so the code is the same on Android and in tests on a normal JVM.
 */
class Identity(val key: PrivateKey, val cert: X509Certificate) {
    /** Lowercase hex SHA-256 of the certificate. */
    val fingerprint: String = fingerprintOf(cert.encoded)

    fun toBytes(): ByteArray {
        val out = ByteArrayOutputStream()
        DataOutputStream(out).use { d ->
            val k = key.encoded
            val c = cert.encoded
            d.writeInt(k.size); d.write(k)
            d.writeInt(c.size); d.write(c)
        }
        return out.toByteArray()
    }

    companion object {
        fun fingerprintOf(der: ByteArray): String =
            MessageDigest.getInstance("SHA-256").digest(der).joinToString("") { "%02x".format(it) }

        fun fromBytes(b: ByteArray): Identity {
            val d = DataInputStream(ByteArrayInputStream(b))
            val k = ByteArray(d.readInt().also { require(it in 1..4096) }).also { d.readFully(it) }
            val c = ByteArray(d.readInt().also { require(it in 1..8192) }).also { d.readFully(it) }
            val key = KeyFactory.getInstance("EC").generatePrivate(PKCS8EncodedKeySpec(k))
            val cert = CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(c)) as X509Certificate
            return Identity(key, cert)
        }

        /** Load the identity from `file`, or make a new one there on the first start. */
        fun loadOrCreate(file: File): Identity {
            runCatching { return fromBytes(file.readBytes()) }
            val id = generate()
            file.parentFile?.mkdirs()
            val tmp = File(file.path + ".tmp")
            tmp.writeBytes(id.toBytes())
            // owner only, where the platform allows it
            runCatching {
                tmp.setReadable(false, false); tmp.setReadable(true, true)
                tmp.setWritable(false, false); tmp.setWritable(true, true)
            }
            if (!tmp.renameTo(file)) {
                file.delete()
                tmp.renameTo(file)
            }
            return id
        }

        fun generate(): Identity {
            val gen = KeyPairGenerator.getInstance("EC")
            gen.initialize(ECGenParameterSpec("secp256r1"), SecureRandom())
            val pair = gen.generateKeyPair()

            val sigAlg = tlv(0x30, oid(0x2A, 0x86.toByte(), 0x48, 0xCE.toByte(), 0x3D, 0x04, 0x03, 0x02)) // ecdsa-with-SHA256
            val name = tlv(0x30, tlv(0x31, tlv(0x30, oid(0x55, 0x04, 0x03) + tlv(0x0C, "Bridge phone".toByteArray()))))
            val serial = ByteArray(8).also { SecureRandom().nextBytes(it); it[0] = (it[0].toInt() and 0x7F).toByte(); if (it[0].toInt() == 0) it[0] = 1 }
            val validity = tlv(0x30, tlv(0x17, "200101000000Z".toByteArray()) + tlv(0x17, "491231235959Z".toByteArray()))
            val basic = tlv(0x30, oid(0x55, 0x1D, 0x13) + tlv(0x01, byteArrayOf(0xFF.toByte())) + tlv(0x04, tlv(0x30, ByteArray(0))))
            val san = tlv(0x30, oid(0x55, 0x1D, 0x11) + tlv(0x04, tlv(0x30, tlv(0x82, "bridge.local".toByteArray()))))
            val extensions = tlv(0xA3, tlv(0x30, basic + san))
            val tbs = tlv(
                0x30,
                tlv(0xA0, tlv(0x02, byteArrayOf(2))) + // version 3
                    tlv(0x02, serial) + sigAlg + name + validity + name + pair.public.encoded + extensions,
            )
            val sig = Signature.getInstance("SHA256withECDSA")
            sig.initSign(pair.private)
            sig.update(tbs)
            val signature = sig.sign()
            val der = tlv(0x30, tbs + sigAlg + tlv(0x03, byteArrayOf(0) + signature))
            val cert = CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(der)) as X509Certificate
            return Identity(pair.private, cert)
        }

        private fun oid(vararg b: Byte) = tlv(0x06, b)

        private fun tlv(tag: Int, content: ByteArray): ByteArray {
            val len = content.size
            val head = ByteArrayOutputStream()
            head.write(tag)
            when {
                len < 0x80 -> head.write(len)
                len < 0x100 -> { head.write(0x81); head.write(len) }
                else -> { head.write(0x82); head.write(len shr 8); head.write(len and 0xFF) }
            }
            return head.toByteArray() + content
        }

        @Suppress("unused")
        private fun big(v: Long) = BigInteger.valueOf(v)
    }
}
