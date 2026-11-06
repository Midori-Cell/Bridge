package dev.bridge.phone.p2p

import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.net.URLDecoder
import java.util.concurrent.SynchronousQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLServerSocket
import javax.net.ssl.SSLSocket
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** A request as the server hands it to the application. `fp` is the fingerprint of the certificate the client showed. */
class Request(
    val method: String,
    val path: String,
    val query: Map<String, String>,
    val headers: Map<String, String>,
    val body: InputStream,
    val contentLength: Long,
    val fp: String?,
    val remote: String,
) {
    /** The body as a JSON object (at most 1 MB); an empty body is an empty object. */
    fun json(): JsonObject {
        if (contentLength <= 0) return JsonObject(emptyMap())
        if (contentLength > 1_000_000) throw Fail(413, "too_large")
        val bytes = ByteArray(contentLength.toInt())
        var n = 0
        while (n < bytes.size) {
            val r = body.read(bytes, n, bytes.size - n)
            if (r < 0) throw Fail(400, "body_error")
            n += r
        }
        return try {
            Json.parseToJsonElement(String(bytes, Charsets.UTF_8)) as? JsonObject ?: throw Fail(400, "bad_json")
        } catch (e: Fail) {
            throw e
        } catch (e: Exception) {
            throw Fail(400, "bad_json")
        }
    }
}

class Answer(val status: Int, val body: JsonObject = JsonObject(emptyMap()))

/** An error the application answers with: status plus a short code, optionally with extra fields. */
class Fail(val status: Int, val code: String, val extra: Map<String, JsonElement> = emptyMap()) : Exception(code)

/** The other side showed a certificate that is not the one expected. */
class PinMismatch : IOException("This is not the device that was paired.")

private const val MAX_HEADER = 16 * 1024

/** Reads one CRLF-terminated line, without the line end. Null at end of stream. */
private fun readLine(input: InputStream): String? {
    val sb = ByteArrayOutputStream()
    while (true) {
        val b = input.read()
        if (b < 0) return if (sb.size() == 0) null else sb.toString("ISO-8859-1")
        if (b == '\n'.code) break
        if (b != '\r'.code) sb.write(b)
        if (sb.size() > MAX_HEADER) throw IOException("line too long")
    }
    return sb.toString("ISO-8859-1")
}

/** A body of known length read from a connection. Reading stops at the end. */
private class Limited(private val input: InputStream, private var left: Long) : InputStream() {
    override fun read(): Int {
        if (left <= 0) return -1
        val b = input.read()
        if (b >= 0) left--
        return b
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int {
        if (left <= 0) return -1
        val n = input.read(b, off, minOf(len.toLong(), left).toInt())
        if (n > 0) left -= n
        return n
    }

    fun skipRest() {
        val buf = ByteArray(4096)
        while (left > 0) if (read(buf, 0, buf.size) < 0) break
    }
}

private fun reason(status: Int) = when (status) {
    200 -> "OK"
    400 -> "Bad Request"
    401 -> "Unauthorized"
    403 -> "Forbidden"
    404 -> "Not Found"
    409 -> "Conflict"
    411 -> "Length Required"
    413 -> "Payload Too Large"
    429 -> "Too Many Requests"
    else -> if (status < 400) "OK" else "Error"
}

/**
 * A small HTTPS server (one request per connection) that asks every client for a certificate. The handler gets the
 * fingerprint of that certificate and decides what it may do. Plain HTTP and clients without a certificate never
 * reach the handler.
 */
class HttpsServer(
    ctx: SSLContext,
    port: Int,
    bind: InetAddress? = null,
    private val handler: (Request) -> Answer,
) {
    private val server: SSLServerSocket = (ctx.serverSocketFactory.createServerSocket() as SSLServerSocket).apply {
        reuseAddress = true
        enabledProtocols = Tls.PROTOCOLS.filter { p -> supportedProtocols.contains(p) }.toTypedArray()
        needClientAuth = true
        bind(InetSocketAddress(bind, port))
    }
    val port: Int get() = server.localPort

    private val pool = ThreadPoolExecutor(0, 48, 30, TimeUnit.SECONDS, SynchronousQueue()) { r ->
        Thread(r, "bridge-peer-http").also { it.isDaemon = true }
    }

    @Volatile
    private var stopped = false

    fun start() {
        Thread({
            while (!stopped) {
                val s = try {
                    server.accept()
                } catch (e: Exception) {
                    if (stopped) return@Thread
                    Thread.sleep(50)
                    continue
                }
                try {
                    pool.execute { serve(s as SSLSocket) }
                } catch (e: Exception) {
                    runCatching { s.close() } // too busy
                }
            }
        }, "bridge-peer-accept").also { it.isDaemon = true }.start()
    }

    fun stop() {
        stopped = true
        runCatching { server.close() }
        pool.shutdownNow()
    }

    private fun serve(s: SSLSocket) {
        try {
            s.soTimeout = 15_000
            s.tcpNoDelay = true
            s.startHandshake()
            val fp = Tls.peerFingerprint(s.session)
            val input = java.io.BufferedInputStream(s.inputStream, 16 * 1024)
            val line = readLine(input) ?: return
            val parts = line.split(' ')
            if (parts.size < 3) return
            val headers = HashMap<String, String>()
            var total = 0
            while (true) {
                val h = readLine(input) ?: return
                if (h.isEmpty()) break
                total += h.length
                if (total > MAX_HEADER) return
                val i = h.indexOf(':')
                if (i > 0) headers[h.substring(0, i).trim().lowercase()] = h.substring(i + 1).trim()
            }
            val rawPath = parts[1]
            val q = rawPath.indexOf('?')
            val path = if (q >= 0) rawPath.substring(0, q) else rawPath
            val query = HashMap<String, String>()
            if (q >= 0) rawPath.substring(q + 1).split('&').filter { it.isNotEmpty() }.forEach {
                val e = it.indexOf('=')
                val k = if (e >= 0) it.substring(0, e) else it
                val v = if (e >= 0) it.substring(e + 1) else ""
                query[URLDecoder.decode(k, "UTF-8")] = URLDecoder.decode(v, "UTF-8")
            }
            val length = headers["content-length"]?.toLongOrNull()
            val answer: Answer
            var body: Limited? = null
            if (headers["transfer-encoding"] != null) {
                answer = fail(411, "length_required")
            } else {
                body = Limited(input, length ?: 0)
                s.soTimeout = 60_000
                answer = try {
                    handler(Request(parts[0].uppercase(), path, query, headers, body, length ?: 0, fp, s.inetAddress?.hostAddress ?: ""))
                } catch (e: Fail) {
                    Answer(e.status, JsonObject(mapOf("error" to JsonPrimitive(e.code)) + e.extra))
                } catch (e: IOException) {
                    throw e
                } catch (e: Exception) {
                    fail(500, "internal")
                }
            }
            runCatching { body?.skipRest() }
            val out = answer.body.toString().toByteArray(Charsets.UTF_8)
            val head = "HTTP/1.1 ${answer.status} ${reason(answer.status)}\r\n" +
                "Content-Type: application/json\r\nContent-Length: ${out.size}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
            s.outputStream.write(head.toByteArray(Charsets.ISO_8859_1))
            s.outputStream.write(out)
            s.outputStream.flush()
        } catch (e: Exception) {
            // a client that went away or never finished its handshake
        } finally {
            runCatching { s.close() }
        }
    }

    private fun fail(status: Int, code: String) = Answer(status, JsonObject(mapOf("error" to JsonPrimitive(code))))
}

/** Lets another thread cut off a request that is running (cancel a transfer, leave a screen). */
class Abort {
    @Volatile
    private var socket: Socket? = null

    @Volatile
    var aborted = false
        private set

    fun abort() {
        aborted = true
        runCatching { socket?.close() }
    }

    internal fun attach(s: Socket) {
        socket = s
        if (aborted) runCatching { s.close() }
    }
}

class Response(val status: Int, val json: JsonObject, val fp: String) {
    val ok: Boolean get() = status in 200..299
    fun error(): String = (json["error"] as? JsonPrimitive)?.content ?: "error"
}

/** The matching client: pinned to a fingerprint when it is known, and always presenting this device's certificate. */
class HttpsClient(private val ctx: SSLContext) {
    /**
     * One request. With `pin`, the connection is dropped before anything is sent unless the other side shows exactly
     * that certificate. The answer carries the fingerprint that was seen (pairing learns it that way).
     */
    fun call(
        host: String,
        port: Int,
        method: String,
        path: String,
        body: ByteArray? = null,
        pin: String? = null,
        connectMs: Int = 3000,
        readMs: Int = 15000,
        abort: Abort? = null,
    ): Response {
        val plain = Socket()
        abort?.attach(plain)
        var ssl: SSLSocket? = null
        try {
            plain.tcpNoDelay = true
            plain.connect(InetSocketAddress(host, port), connectMs)
            plain.soTimeout = connectMs.coerceAtLeast(5000)
            ssl = ctx.socketFactory.createSocket(plain, host, port, true) as SSLSocket
            ssl.enabledProtocols = Tls.PROTOCOLS.filter { p -> ssl.supportedProtocols.contains(p) }.toTypedArray()
            ssl.startHandshake()
            val fp = Tls.peerFingerprint(ssl.session) ?: throw IOException("no certificate")
            if (pin != null && !constantTimeEquals(pin, fp)) throw PinMismatch()
            ssl.soTimeout = readMs
            val payload = body ?: ByteArray(0)
            val hostHeader = if (host.contains(':')) "[$host]:$port" else "$host:$port"
            val head = StringBuilder("$method $path HTTP/1.1\r\nHost: $hostHeader\r\nAccept: application/json\r\nConnection: close\r\n")
            if (body != null) head.append("Content-Type: application/json\r\n")
            head.append("Content-Length: ${payload.size}\r\n\r\n")
            val out = ssl.outputStream
            out.write(head.toString().toByteArray(Charsets.ISO_8859_1))
            out.write(payload)
            out.flush()

            val input = java.io.BufferedInputStream(ssl.inputStream, 16 * 1024)
            val status = readLine(input)?.split(' ')?.getOrNull(1)?.toIntOrNull() ?: throw IOException("bad answer")
            val headers = HashMap<String, String>()
            while (true) {
                val h = readLine(input) ?: throw IOException("bad answer")
                if (h.isEmpty()) break
                val i = h.indexOf(':')
                if (i > 0) headers[h.substring(0, i).trim().lowercase()] = h.substring(i + 1).trim()
            }
            val bytes = ByteArrayOutputStream()
            val len = headers["content-length"]?.toLongOrNull()
            if (headers["transfer-encoding"]?.contains("chunked", true) == true) {
                while (true) {
                    val size = readLine(input)?.substringBefore(';')?.trim()?.toIntOrNull(16) ?: throw IOException("bad chunk")
                    if (size == 0) break
                    if (bytes.size() + size > 1_000_000) throw IOException("answer too large")
                    val chunk = ByteArray(size)
                    var n = 0
                    while (n < size) {
                        val r = input.read(chunk, n, size - n)
                        if (r < 0) throw IOException("short answer")
                        n += r
                    }
                    bytes.write(chunk)
                    readLine(input)
                }
            } else if (len != null) {
                if (len > 1_000_000) throw IOException("answer too large")
                val buf = ByteArray(len.toInt())
                var n = 0
                while (n < buf.size) {
                    val r = input.read(buf, n, buf.size - n)
                    if (r < 0) throw IOException("short answer")
                    n += r
                }
                bytes.write(buf)
            } else {
                val buf = ByteArray(4096)
                while (true) {
                    val r = input.read(buf)
                    if (r < 0) break
                    bytes.write(buf, 0, r)
                    if (bytes.size() > 1_000_000) throw IOException("answer too large")
                }
            }
            val json = try {
                Json.parseToJsonElement(bytes.toString("UTF-8")) as? JsonObject ?: JsonObject(emptyMap())
            } catch (e: Exception) {
                JsonObject(emptyMap())
            }
            return Response(status, json, fp)
        } catch (e: SocketTimeoutException) {
            throw IOException("The other device did not answer in time.", e)
        } finally {
            runCatching { (ssl ?: plain).close() }
            runCatching { plain.close() }
        }
    }

    companion object {
        fun constantTimeEquals(a: String, b: String): Boolean {
            if (a.length != b.length) return false
            var d = 0
            for (i in a.indices) d = d or (a[i].code xor b[i].code)
            return d == 0
        }
    }
}
