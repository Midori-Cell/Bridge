package dev.bridge.phone.p2p

import java.util.Base64

/**
 * The small preview that travels with a picture offer (a JPEG of about 48 pixels, as base64 text). A preview that comes
 * from another device is only kept when it is a small, well-formed JPEG; anything else is dropped without failing the offer.
 */
object Thumb {
    /** Longest side of a preview that is made here, in pixels. */
    const val EDGE = 48
    const val MAX_B64 = 16_000
    private const val MAX_SIDE = 160

    fun valid(b64: String): Boolean {
        if (b64.isEmpty() || b64.length > MAX_B64) return false
        val bytes = try {
            Base64.getDecoder().decode(b64)
        } catch (e: IllegalArgumentException) {
            return false
        }
        if (bytes.size < 4 || bytes[0] != 0xFF.toByte() || bytes[1] != 0xD8.toByte() || bytes[2] != 0xFF.toByte()) return false
        val (w, h) = dimensions(bytes) ?: return false
        return w in 1..MAX_SIDE && h in 1..MAX_SIDE
    }

    /** Width and height from the first "start of frame" marker of a JPEG, or null if there is none. */
    fun dimensions(b: ByteArray): Pair<Int, Int>? {
        var i = 2
        while (i + 9 < b.size) {
            if (b[i] != 0xFF.toByte()) return null
            val marker = b[i + 1].toInt() and 0xFF
            if (marker == 0xFF) { i++; continue } // padding
            if (marker == 0xD8 || marker == 0x01 || marker in 0xD0..0xD7) { i += 2; continue } // markers without a length
            val len = ((b[i + 2].toInt() and 0xFF) shl 8) or (b[i + 3].toInt() and 0xFF)
            val sof = marker in 0xC0..0xCF && marker != 0xC4 && marker != 0xC8 && marker != 0xCC
            if (sof) {
                val h = ((b[i + 5].toInt() and 0xFF) shl 8) or (b[i + 6].toInt() and 0xFF)
                val w = ((b[i + 7].toInt() and 0xFF) shl 8) or (b[i + 8].toInt() and 0xFF)
                return w to h
            }
            if (len < 2) return null
            i += 2 + len
        }
        return null
    }

    fun encode(jpeg: ByteArray): String = Base64.getEncoder().encodeToString(jpeg)

    fun decode(b64: String): ByteArray? = try {
        Base64.getDecoder().decode(b64)
    } catch (e: IllegalArgumentException) {
        null
    }
}
