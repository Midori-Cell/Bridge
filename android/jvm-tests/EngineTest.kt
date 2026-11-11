import dev.bridge.phone.*
import dev.bridge.phone.p2p.*
import java.io.File
import java.net.InetAddress


/** A picture file (PNG) and the preview a phone would make for it. */
class PicSource(private val file: File, private val thumbText: String?) : FileSource {
    override val name: String get() = file.name
    override val size: Long get() = file.length()
    override fun open(offset: Long): java.io.InputStream = java.io.FileInputStream(file).also { it.skipNBytes(offset) }
    override fun thumb(): String? = thumbText
}

fun makePicture(file: File, w: Int, h: Int) {
    val img = java.awt.image.BufferedImage(w, h, java.awt.image.BufferedImage.TYPE_INT_RGB)
    for (x in 0 until w) for (y in 0 until h) img.setRGB(x, y, ((x * 255 / w) shl 16) or ((y * 255 / h) shl 8) or 90)
    javax.imageio.ImageIO.write(img, "png", file)
}

fun jpegPreview(w: Int, h: Int): String {
    val img = java.awt.image.BufferedImage(w, h, java.awt.image.BufferedImage.TYPE_INT_RGB)
    for (x in 0 until w) for (y in 0 until h) img.setRGB(x, y, (x * 5 shl 16) or (y * 5 shl 8) or 60)
    val out = java.io.ByteArrayOutputStream()
    javax.imageio.ImageIO.write(img, "jpg", out)
    return Thumb.encode(out.toByteArray())
}

fun check(c: Boolean, m: String) { if (!c) throw AssertionError(m) }
fun waitFor(what: String, ms: Long = 15000, f: () -> Boolean) {
    val end = System.currentTimeMillis() + ms
    while (System.currentTimeMillis() < end) { if (f()) return; Thread.sleep(50) }
    throw AssertionError("timeout: $what")
}
fun tmp(n: String) = File.createTempFile("bt-" + n, "").also { it.delete(); it.mkdirs() }
val lo: InetAddress = InetAddress.getLoopbackAddress()

class Dev(val dir: File, val name: String, port: Int = 0, val auto: Boolean = false, val finder: Finder = Finder { emptyList() }) {
    val id = Identity.loadOrCreate(File(dir, "id.bin"))
    val received = java.util.concurrent.CopyOnWriteArrayList<File>()
    val texts = java.util.concurrent.CopyOnWriteArrayList<String>()
    val e = PeerEngine(dir, id, { name }, finder, { null }, { auto }, {}, { _, _, f -> received.add(f) }, lo, port, onText = { _, from, t -> texts.add("$from: $t") })
    val port = e.start()
}

fun main() {
    val da = tmp("a"); val db = tmp("b")
    var a = Dev(da, "Alice phone")
    var b = Dev(db, "Bob phone")

    // --- pairing: A asks B
    val op = a.e.pairStart("127.0.0.1", b.port)
    waitFor("B sees request") { b.e.pairRequests().isNotEmpty() }
    val inc = b.e.pairRequests()[0]
    check(inc.code == op.code, "codes differ: ${inc.code} vs ${op.code}")
    check(Regex("\\d{3} \\d{3}").matches(op.code), "format ${op.code}")
    check(inc.name == "Alice phone", "name ${inc.name}")
    check(a.e.peers().isEmpty() && b.e.peers().isEmpty(), "nobody trusted yet")
    // A's user confirms first: still nothing is saved
    check(a.e.pairConfirm(op.id), "confirm")
    Thread.sleep(300)
    check(b.e.peers().isEmpty() && a.e.peers().isEmpty(), "not before both")
    check(b.e.pairAccept(inc.id), "accept")
    waitFor("A done") { a.e.outPairs().firstOrNull()?.state == PairState.Done }
    check(a.e.peers().map { it.fp } == listOf(b.id.fingerprint), "A has B")
    check(b.e.peers().map { it.fp } == listOf(a.id.fingerprint), "B has A")
    check(a.e.peers()[0].name == "Bob phone" && b.e.peers()[0].name == "Alice phone", "names")
    println("pairing ok, code ${op.code}")

    // --- declined pairing does not pair
    val c = Dev(tmp("c"), "Carol")
    val op2 = c.e.pairStart("127.0.0.1", b.port)
    waitFor("req") { b.e.pairRequests().any { it.name == "Carol" } }
    b.e.pairDecline(b.e.pairRequests().first { it.name == "Carol" }.id)
    waitFor("declined") { c.e.outPairs().first().state == PairState.Declined }
    check(b.e.peers().size == 1, "carol not paired")
    // a stranger (not paired) cannot send messages
    val stranger = HttpsClient(Tls.context(c.id))
    val r = stranger.call("127.0.0.1", b.port, "POST", "/peer/messages", "{\"text\":\"hi\"}".toByteArray())
    check(r.status == 403, "stranger ${r.status}")
    println("decline + stranger ok")

    // --- messages
    val bfp = b.id.fingerprint; val afp = a.id.fingerprint
    val n1 = a.e.sendText(bfp, "hello Bob")
    check(n1 != null && !n1.pending, "sent")
    waitFor("B got it") { b.e.notes(afp).any { it.text == "hello Bob" } }
    val got = b.e.notes(afp).first { it.text == "hello Bob" }
    check(b.texts.toList() == listOf("Alice phone: hello Bob"), "text callback ${b.texts}")
    check(got.fromPc && got.mid == n1!!.mid, "side + mid")
    b.e.sendText(afp, "hi Alice", got.mid)
    waitFor("A got reply") { a.e.notes(bfp).any { it.text == "hi Alice" } }
    val rep = a.e.notes(bfp).first { it.text == "hi Alice" }
    check(rep.reply?.mid == got.mid && rep.reply?.preview == "hello Bob", "reply quote ${rep.reply}")
    check(rep.reply?.src == "Alice phone", "quote is of my message, src=${rep.reply?.src}")
    // delete for both
    a.e.deleteNote(bfp, n1!!)
    waitFor("deleted on B") { b.e.notes(afp).none { it.text == "hello Bob" } }
    check(a.e.notes(bfp).none { it.text == "hello Bob" }, "deleted on A")
    println("messages ok")

    // --- the other side is away: texts wait and go out in order when it is back
    b.e.stop(); Thread.sleep(300)
    val w1 = a.e.sendText(bfp, "while you were away 1")
    val w2 = a.e.sendText(bfp, "while you were away 2")
    check(w1 == null && w2 == null, "queued")
    check(a.e.notes(bfp).filter { it.pending }.map { it.text } == listOf("while you were away 1", "while you were away 2"), "shown as waiting")
    // survives a restart of A
    a.e.stop(); a = Dev(da, "Alice phone", a.port)
    check(a.e.notes(bfp).count { it.pending } == 2, "queue persisted")
    b = Dev(db, "Bob phone", b.port)
    waitFor("delivered after B is back", 20000) { b.e.notes(afp).count { it.text?.startsWith("while you were away") == true } == 2 }
    check(b.e.notes(afp).filter { it.text?.startsWith("while") == true }.map { it.text } == listOf("while you were away 1", "while you were away 2"), "order")
    waitFor("no longer pending") { a.e.notes(bfp).none { it.pending } }
    println("offline queue ok")

    // --- files
    val src = File(da, "photo.bin"); val data = ByteArray(9_000_000) { (it * 31 + 7).toByte() }; src.writeBytes(data)
    val tid = a.e.sendFile(bfp, FileOnDisk(src), null)
    waitFor("offer appears") { b.e.transfers().any { it.id == tid && it.state == XState.Offered } }
    check(a.e.transfers().first { it.id == tid }.state == XState.Offered, "sender waits for answer")
    check(b.e.acceptFile(tid), "accept file")
    waitFor("file done", 30000) { a.e.transfers().firstOrNull { it.id == tid }?.state == XState.Done && b.e.transfers().firstOrNull { it.id == tid }?.state == XState.Done }
    val f = b.e.inboxFile(tid)!!
    check(f.readBytes().contentEquals(data), "content")
    check(b.received.size == 1, "callback")
    check(b.e.notes(afp).any { it.isFile && it.file?.dir == "to_phone" && it.file?.name == "photo.bin" }, "note on B")
    check(a.e.notes(bfp).any { it.isFile && it.file?.dir == "to_pc" }, "note on A")
    println("file ok")

    // --- pictures: a small preview goes with the offer, nothing else is downloaded before the user accepts
    val pic = File(da, "view.png"); makePicture(pic, 200, 120)
    val preview = jpegPreview(48, 29)
    check(Thumb.valid(preview), "own preview is valid")
    val ptid = a.e.sendFile(bfp, PicSource(pic, preview), null)
    waitFor("picture offer on B") { b.e.transfers().any { it.id == ptid && it.state == XState.Offered } }
    check(b.e.transfers().first { it.id == ptid }.thumb == preview, "preview arrived with the offer")
    check(a.e.transfers().first { it.id == ptid }.thumb == preview, "sender keeps it too")
    check(b.e.inboxFile(ptid) == null, "nothing downloaded yet")
    check(b.e.acceptFile(ptid), "accept picture")
    waitFor("picture done") { b.e.transfers().firstOrNull { it.id == ptid }?.state == XState.Done && a.e.transfers().firstOrNull { it.id == ptid }?.state == XState.Done }
    check(b.e.notes(afp).first { it.file?.id == ptid }.file?.thumb == preview, "preview kept in the chat note on B")
    check(a.e.notes(bfp).first { it.file?.id == ptid }.file?.thumb == preview, "and on A")
    // survives a restart (it is in the saved history)
    b.e.stop(); b = Dev(db, "Bob phone", b.port)
    check(b.e.notes(afp).first { it.file?.id == ptid }.file?.thumb == preview, "preview persisted")
    // a preview that is not a small JPEG is dropped; the offer still works
    val pic2 = File(da, "odd.png"); makePicture(pic2, 20, 20)
    for (bad in listOf("AAAA", "not base64!!", Thumb.encode(ByteArray(100) { 1 }), jpegPreview(300, 300))) {
        val t2 = a.e.sendFile(bfp, PicSource(pic2, bad), null)
        waitFor("odd offer on B") { b.e.transfers().any { it.id == t2 && it.state == XState.Offered } }
        check(b.e.transfers().first { it.id == t2 }.thumb == null, "bad preview dropped: ${bad.take(12)}")
        b.e.declineFile(t2)
        waitFor("declined") { a.e.transfers().first { it.id == t2 }.state == XState.Declined }
    }
    println("pictures ok")

    // declined file
    val tid2 = a.e.sendFile(bfp, FileOnDisk(src), null)
    waitFor("offer 2") { b.e.transfers().any { it.id == tid2 && it.state == XState.Offered } }
    b.e.declineFile(tid2)
    waitFor("declined on A") { a.e.transfers().first { it.id == tid2 }.state == XState.Declined }
    // cancelled by the sender while waiting
    val tid3 = a.e.sendFile(bfp, FileOnDisk(src), null)
    waitFor("offer 3") { b.e.transfers().any { it.id == tid3 && it.state == XState.Offered } }
    a.e.cancelTransfer(tid3)
    waitFor("cancelled on A") { a.e.transfers().first { it.id == tid3 }.state == XState.Cancelled }
    waitFor("cancelled on B") { b.e.transfers().first { it.id == tid3 }.state == XState.Cancelled }
    // file for an away device waits (needs a ref to reopen)
    b.e.stop(); Thread.sleep(300)
    val tid4 = a.e.sendFile(bfp, FileOnDisk(src), "ref")
    waitFor("queued") { a.e.transfers().first { it.id == tid4 }.state == XState.Queued }
    println("file decline/cancel/queue ok")

    // --- unpair
    b = Dev(db, "Bob phone", b.port)
    a.e.unpair(bfp)
    waitFor("B forgot A") { b.e.peers().isEmpty() }
    check(a.e.peers().isEmpty() && a.e.notes(bfp).isEmpty(), "A forgot B")
    val r2 = a.e.let { HttpsClient(Tls.context(a.id)).call("127.0.0.1", b.port, "POST", "/peer/messages", "{\"text\":\"x\"}".toByteArray()) }
    check(r2.status == 403, "no longer trusted")
    println("ALL OK")
    System.exit(0)
}
