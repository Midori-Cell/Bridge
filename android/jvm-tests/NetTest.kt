import dev.bridge.phone.p2p.*
import kotlinx.serialization.json.*
import java.io.File
fun main() {
    val a = Identity.generate(); val b = Identity.generate()
    check(a.fingerprint.length == 64 && a.fingerprint != b.fingerprint)
    val f = File.createTempFile("idtest", ".bin"); f.delete()
    val a2 = Identity.loadOrCreate(f); check(Identity.loadOrCreate(f).fingerprint == a2.fingerprint)
    val srv = HttpsServer(Tls.context(a), 0, java.net.InetAddress.getLoopbackAddress()) { r ->
        Answer(200, buildJsonObject { put("path", r.path); put("fp", r.fp ?: "none"); put("q", r.query["x"] ?: ""); put("len", r.contentLength) })
    }
    srv.start()
    val c = HttpsClient(Tls.context(b))
    val r = c.call("127.0.0.1", srv.port, "POST", "/hello/there?x=a%20b", "{\"k\":1}".toByteArray(), pin = a.fingerprint)
    println(r.status.toString() + " " + r.json)
    check(r.status == 200 && r.json["fp"]!!.jsonPrimitive.content == b.fingerprint && r.fp == a.fingerprint)
    try { c.call("127.0.0.1", srv.port, "GET", "/", pin = b.fingerprint); error("no pin check") } catch (e: PinMismatch) { println("pin ok") }
    srv.stop()
    println("net ok")
}
