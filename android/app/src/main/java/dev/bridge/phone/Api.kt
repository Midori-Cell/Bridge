package dev.bridge.phone

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.serialization.DeserializationStrategy
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put
import okhttp3.Call
import okhttp3.Callback
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okio.BufferedSink
import java.io.IOException
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** The PC answered with an error. `code` is the short machine-readable text from the PC ("declined", "disk_full", ...). */
class ApiException(val status: Int, val code: String) : IOException("$code ($status)")

/** Something went wrong that can be told to the user as it is (not a network problem, so nothing is retried). */
class Problem(message: String) : Exception(message)

val JSON = Json { ignoreUnknownKeys = true; coerceInputValues = true }

fun fingerprintOf(der: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(der).joinToString("") { "%02x".format(it) }

/**
 * HTTPS to a PC. The PC has a self-signed certificate, so no certificate authority is involved:
 * the only thing trusted is the certificate whose fingerprint was in the QR code. With `fingerprint == null`
 * (pairing by typing the address) any certificate is accepted once and reported through `onSeen`.
 */
fun pinnedClient(fingerprint: String?, onSeen: ((String) -> Unit)? = null): OkHttpClient {
    val trust = object : X509TrustManager {
        override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {
            throw CertificateException("This app does not use client certificates.")
        }

        override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
            val seen = fingerprintOf(chain[0].encoded)
            onSeen?.invoke(seen)
            if (fingerprint != null && !seen.equals(fingerprint, ignoreCase = true)) {
                throw CertificateException("This is not the PC that was paired.")
            }
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
    }
    val context = SSLContext.getInstance("TLS")
    context.init(null, arrayOf(trust), SecureRandom())
    return OkHttpClient.Builder()
        .sslSocketFactory(context.socketFactory, trust)
        .hostnameVerifier { _, _ -> true } // the certificate is pinned, the host name is irrelevant
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(45, TimeUnit.SECONDS) // a long poll waits up to 30 s
        .writeTimeout(60, TimeUnit.SECONDS)
        .build()
}

suspend fun Call.await(): Response = suspendCancellableCoroutine { cont ->
    cont.invokeOnCancellation { cancel() }
    enqueue(object : Callback {
        override fun onFailure(call: Call, e: IOException) {
            if (cont.isActive) cont.resumeWithException(e)
        }

        override fun onResponse(call: Call, response: Response) {
            if (cont.isActive) cont.resume(response) else response.close()
        }
    })
}

/** Where a PC is: `https://host:port` (an IPv6 address needs brackets). */
fun baseUrl(host: String, port: Int): String = if (host.contains(':')) "https://[$host]:$port" else "https://$host:$port"

private val JSON_TYPE = "application/json".toMediaType()
private val BINARY_TYPE = "application/octet-stream".toMediaType()

private fun errorCode(body: String): String =
    runCatching { (JSON.parseToJsonElement(body) as JsonObject)["error"]?.let { (it as JsonPrimitive).contentOrNull } }.getOrNull() ?: "error"

/** A request body that reports how much has been written, so a progress bar can follow a big chunk. */
private class ProgressBody(private val bytes: ByteArray, private val onProgress: (Long) -> Unit) : RequestBody() {
    override fun contentType() = BINARY_TYPE
    override fun contentLength() = bytes.size.toLong()
    override fun writeTo(sink: BufferedSink) {
        var offset = 0
        while (offset < bytes.size) {
            val n = minOf(64 * 1024, bytes.size - offset)
            sink.write(bytes, offset, n)
            sink.flush()
            offset += n
            onProgress(offset.toLong())
        }
    }
}

class Api(private val pc: Pc, private val http: OkHttpClient) {
    private val base = baseUrl(pc.host, pc.port)

    private fun request(path: String): Request.Builder = Request.Builder().url(base + path).header("Authorization", "Bearer ${pc.token}")

    private suspend fun <T> call(request: Request, reader: DeserializationStrategy<T>, client: OkHttpClient = http): T {
        client.newCall(request).await().use { response ->
            val body = response.body?.string().orEmpty()
            if (!response.isSuccessful) throw ApiException(response.code, errorCode(body))
            return JSON.decodeFromString(reader, body)
        }
    }

    private fun jsonBody(text: String) = text.toRequestBody(JSON_TYPE)

    suspend fun ping(): PingResponse = call(request("/api/ping").build(), PingResponse.serializer())

    /** The history (`since == null`) or the news since a note id, waiting up to `wait` seconds for something to happen. */
    suspend fun notes(since: Long?, del: Long?, wait: Int): NotesResponse {
        val path = if (since == null) "/api/notes?wait=0" else "/api/notes?since=$since&wait=$wait&del=${del ?: 0}"
        // a PC that vanished (Wi-Fi off) should be noticed within a few seconds after the longest wait, not after 45
        val client = if (since == null) http else http.newBuilder().readTimeout(wait + 8L, TimeUnit.SECONDS).build()
        return call(request(path).build(), NotesResponse.serializer(), client)
    }

    suspend fun sendNote(text: String, replyTo: String?): Note {
        val body = buildJsonObject {
            put("text", text)
            if (replyTo != null) put("reply_to", replyTo)
        }
        return call(request("/api/notes").post(jsonBody(body.toString())).build(), NoteResponse.serializer()).note
    }

    suspend fun deleteNote(mid: String) {
        http.newCall(request("/api/notes/$mid").delete().build()).await().use {
            if (!it.isSuccessful && it.code != 404) throw ApiException(it.code, errorCode(it.body?.string().orEmpty()))
        }
    }

    suspend fun unpair() {
        http.newCall(request("/api/unpair").post(jsonBody("{}")).build()).await().use { }
    }

    // ---- phone -> PC

    suspend fun createFile(name: String, size: Long): CreateResponse {
        val body = buildJsonObject {
            put("name", name)
            put("size", size)
        }
        return call(request("/api/files").post(jsonBody(body.toString())).build(), CreateResponse.serializer())
    }

    suspend fun fileStatus(id: String): StatusResponse = call(request("/api/files/$id").build(), StatusResponse.serializer())

    suspend fun putChunk(id: String, offset: Long, bytes: ByteArray, onProgress: (Long) -> Unit): PutResponse =
        call(request("/api/files/$id?offset=$offset").put(ProgressBody(bytes, onProgress)).build(), PutResponse.serializer())

    suspend fun cancelFile(id: String) {
        http.newCall(request("/api/files/$id").delete().build()).await().use { }
    }

    // ---- PC -> phone

    /** Stream a file from the PC, starting at byte `from`. `onChunk` gets each piece. */
    suspend fun download(id: String, from: Long, onChunk: (ByteArray, Int) -> Unit) {
        val builder = request("/api/files/$id/content")
        if (from > 0) builder.header("Range", "bytes=$from-")
        http.newCall(builder.build()).await().use { response ->
            if (!response.isSuccessful) throw ApiException(response.code, errorCode(response.body?.string().orEmpty()))
            if (from > 0 && response.code != 206) throw IOException("The PC did not continue the download.")
            val input = response.body!!.byteStream()
            val buffer = ByteArray(64 * 1024)
            while (true) {
                currentCoroutineContext().ensureActive()
                val n = input.read(buffer)
                if (n < 0) break
                onChunk(buffer, n)
            }
        }
    }

    companion object {
        /** Pair with the PC at `host:port` using the code from its screen. Returns what the PC answered and the certificate it showed. */
        suspend fun pair(host: String, port: Int, code: String, phoneName: String, fingerprint: String?): Pair<PairResponse, String> {
            var seen = ""
            val client = pinnedClient(fingerprint) { seen = it }
            val body = buildJsonObject {
                put("code", code)
                put("name", phoneName)
            }
            val request = Request.Builder().url(baseUrl(host, port) + "/api/pair").post(body.toString().toRequestBody(JSON_TYPE)).build()
            client.newCall(request).await().use { response ->
                val text = response.body?.string().orEmpty()
                if (!response.isSuccessful) throw ApiException(response.code, errorCode(text))
                return JSON.decodeFromString(PairResponse.serializer(), text) to seen
            }
        }
    }
}

/** `https://host:port/#pair=CODE&fp=FINGERPRINT`, the content of the QR code on the PC. */
data class PairLink(val host: String, val port: Int, val code: String, val fingerprint: String)

fun parsePairLink(text: String): PairLink? {
    val url = text.trim().toHttpUrlOrNull() ?: return null
    if (url.scheme != "https") return null
    val parts = (url.fragment ?: return null).split('&').mapNotNull { p -> p.split('=', limit = 2).takeIf { it.size == 2 }?.let { it[0] to it[1] } }.toMap()
    val code = parts["pair"] ?: return null
    val fp = parts["fp"]?.lowercase() ?: return null
    if (code.isBlank() || !Regex("[0-9a-f]{64}").matches(fp)) return null
    return PairLink(url.host, url.port, code, fp)
}
