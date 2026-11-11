package dev.bridge.phone

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ImageDecoder
import android.net.Uri
import android.util.LruCache
import dev.bridge.phone.p2p.FileOnDisk
import dev.bridge.phone.p2p.FileSource
import dev.bridge.phone.p2p.Thumb
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * Pictures in the chat. A picture that is sent carries a tiny preview (about 48 pixels); the receiver shows it enlarged,
 * which makes it blurred, until the picture itself was downloaded. Everything here works on a background thread.
 */
object Thumbs {
    private const val SHOWN_EDGE = 720
    private const val KEPT_EDGE = 640

    private val cache = object : LruCache<String, Bitmap>(24 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.byteCount
    }

    fun isPicture(mime: String?): Boolean = mime != null && mime.startsWith("image/") && !mime.contains("svg")

    private fun decode(source: ImageDecoder.Source, edge: Int): Bitmap? = try {
        ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
            val w = info.size.width
            val h = info.size.height
            val scale = edge.toFloat() / max(w, h)
            if (scale < 1f) decoder.setTargetSize(max(1, (w * scale).roundToInt()), max(1, (h * scale).roundToInt()))
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE // the bitmap is compressed and drawn, so it must be readable
        }
    } catch (e: Exception) {
        null // not a picture the phone can read
    } catch (e: OutOfMemoryError) {
        null
    }

    /** A JPEG on a white background (transparent pictures would turn black otherwise). */
    private fun jpeg(bitmap: Bitmap, quality: Int): ByteArray {
        val flat = Bitmap.createBitmap(bitmap.width, bitmap.height, Bitmap.Config.ARGB_8888)
        Canvas(flat).apply {
            drawColor(Color.WHITE)
            drawBitmap(bitmap, 0f, 0f, null)
        }
        val out = ByteArrayOutputStream()
        flat.compress(Bitmap.CompressFormat.JPEG, quality, out)
        flat.recycle()
        return out.toByteArray()
    }

    private fun preview(source: ImageDecoder.Source): String? {
        val bitmap = decode(source, Thumb.EDGE) ?: return null
        val text = Thumb.encode(jpeg(bitmap, 60))
        return text.takeIf { Thumb.valid(it) }
    }

    /** The preview that goes with an offer, or null if this is not a picture. */
    fun make(context: Context, uri: Uri): String? =
        if (isPicture(context.contentResolver.getType(uri))) preview(ImageDecoder.createSource(context.contentResolver, uri)) else null

    fun make(file: File): String? =
        if (isPicture(Bridge.mimeOf(file.name))) preview(ImageDecoder.createSource(file)) else null

    // ---------------------------------------------------------------- the picture of a file that was sent from this phone

    private fun keptFile(context: Context, id: String): File = File(File(context.filesDir, "previews"), "$id.jpg")

    /** Keep a larger copy of a picture that is sent, so the chat can show it sharp. */
    fun keep(context: Context, id: String, source: ImageDecoder.Source) {
        try {
            val bitmap = decode(source, KEPT_EDGE) ?: return
            val file = keptFile(context, id)
            file.parentFile?.mkdirs()
            file.writeBytes(jpeg(bitmap, 80))
        } catch (e: Exception) {
            // the chat then keeps showing the small preview
        }
    }

    fun keep(context: Context, id: String, uri: Uri) = keep(context, id, ImageDecoder.createSource(context.contentResolver, uri))

    fun keep(context: Context, id: String, file: File) = keep(context, id, ImageDecoder.createSource(file))

    // ---------------------------------------------------------------- showing

    /** The small preview of an offer, enlarged by the screen: blurred. */
    fun small(b64: String): Bitmap? {
        val bytes = Thumb.decode(b64) ?: return null
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
    }

    /** The picture itself, if this phone has it: a copy it sent, or a download in Downloads/Bridge. */
    fun sharp(context: Context, id: String, savedUri: String?): Bitmap? {
        val key = id + "|" + (savedUri ?: "")
        cache.get(key)?.let { return it }
        val kept = keptFile(context, id)
        val bitmap = when {
            kept.isFile -> decode(ImageDecoder.createSource(kept), SHOWN_EDGE)
            savedUri != null -> decode(ImageDecoder.createSource(context.contentResolver, Uri.parse(savedUri)), SHOWN_EDGE)
            else -> null
        }
        if (bitmap != null) cache.put(key, bitmap)
        return bitmap
    }
}

/** A file from the phone's storage, with a preview if it is a picture. */
class PictureFile(private val context: Context, private val file: File) : FileSource by FileOnDisk(file) {
    override fun thumb(): String? = Thumbs.make(file)
    override fun keepPreview(id: String) = Thumbs.keep(context, id, file)
}
