import dev.bridge.phone.*
import dev.bridge.phone.p2p.*
import java.io.*
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
fun waitFor(what: String, ms: Long = 20000, f: () -> Boolean) {
    val end = System.currentTimeMillis() + ms
    while (System.currentTimeMillis() < end) { if (f()) return; Thread.sleep(80) }
    throw AssertionError("timeout: $what")
}
fun tmp(n: String) = File.createTempFile("bt-" + n, "").also { it.delete(); it.mkdirs() }
val lo: InetAddress = InetAddress.getLoopbackAddress()

class Pc(exe: String) {
    val dir = tmp("pcdata"); val inbox = tmp("pcinbox")
    val p = ProcessBuilder(exe, "Desk PC", dir.path, inbox.path, "auto").redirectError(ProcessBuilder.Redirect.INHERIT).start()
    val r = BufferedReader(InputStreamReader(p.inputStream)); val w = PrintWriter(p.outputStream, true)
    val port: Int; val fp: String
    init { val l = r.readLine().split(" "); check(l[0] == "READY", "ready: $l"); port = l[1].toInt(); fp = l[2] }
    @Synchronized fun one(c: String): String { w.println(c); return r.readLine() }
    @Synchronized fun many(c: String): List<String> { w.println(c); val o = ArrayList<String>(); while (true) { val l = r.readLine(); if (l == "END") return o; o.add(l) } }
    fun quit() { w.println("quit"); p.waitFor(5, java.util.concurrent.TimeUnit.SECONDS); p.destroy() }
}

class Phone(val dir: File, val name: String, port: Int = 0, val auto: Boolean = false) {
    val id = Identity.loadOrCreate(File(dir, "id.bin"))
    val received = java.util.concurrent.CopyOnWriteArrayList<File>()
    val e = PeerEngine(dir, id, { name }, { emptyList() }, { null }, { auto }, {}, { _, _, f -> received.add(f) }, lo, port)
    val port = e.start()
}

fun main(args: Array<String>) {
    val pc = Pc(args[0])
    val dp = tmp("phone")
    var ph = Phone(dp, "Pixel")
    val pcfp = pc.fp

    // --- phone asks the PC
    val op = ph.e.pairStart("127.0.0.1", pc.port)
    val req = pc.many("requests")
    check(req.size == 1 && req[0].contains(op.code) && req[0].endsWith("| Pixel"), "PC request: $req code=${op.code}")
    ph.e.pairConfirm(op.id)
    val reqId = req[0].split(" ")[1]
    check(pc.one("accept $reqId") == "OK true", "accept")
    waitFor("phone paired") { ph.e.outPairs().first().state == PairState.Done }
    waitFor("pc has phone") { pc.many("peers").any { it.contains(ph.id.fingerprint) && it.endsWith("Pixel") } }
    check(ph.e.peers()[0].fp == pcfp && ph.e.peers()[0].name == "Desk PC", "phone has PC")
    println("phone->PC pairing ok (${op.code})")

    // --- messages both ways, quotes both ways
    val n1 = ph.e.sendText(pcfp, "hi from the phone")!!
    waitFor("pc text") { pc.many("notes ${ph.id.fingerprint}").any { it.contains("hi from the phone") } }
    val pfp = ph.id.fingerprint
    val line = pc.many("notes $pfp").first { it.contains("hi from the phone") }
    check(line.split(" ")[1] == n1.mid, "mid kept: $line")
    val r1 = pc.one("reply $pfp ${n1.mid} answer from the PC")
    check(r1.startsWith("OK"), r1)
    waitFor("phone got reply") { ph.e.notes(pcfp).any { it.text == "answer from the PC" } }
    val rep = ph.e.notes(pcfp).first { it.text == "answer from the PC" }
    check(rep.reply?.mid == n1.mid && rep.reply?.src == "Pixel" && rep.reply?.preview == "hi from the phone", "quote on phone: ${rep.reply}")
    ph.e.sendText(pcfp, "and a reply to you", rep.mid)
    waitFor("pc got phone reply") { pc.many("notes $pfp").any { it.contains("and a reply to you") && it.contains("|| ${rep.mid}:") && it.endsWith("answer from the PC") } }
    println("messages + quotes ok")

    // --- delete both ways
    ph.e.deleteNote(pcfp, n1)
    waitFor("deleted on PC") { pc.many("notes $pfp").none { it.split(" ")[1] == n1.mid } }
    val mine = pc.one("text $pfp to be deleted by the PC").split(" ")[1]
    waitFor("phone got it") { ph.e.notes(pcfp).any { it.mid == mine } }
    check(pc.one("delete $pfp $mine") == "OK true", "pc delete")
    waitFor("deleted on phone") { ph.e.notes(pcfp).none { it.mid == mine } }
    println("delete ok")

    // --- files: phone -> PC (the PC accepts automatically)
    val data = ByteArray(6_000_000) { (it * 13 + 5).toByte() }
    val src = File(dp, "report.bin"); src.writeBytes(data)
    val tid = ph.e.sendFile(pcfp, FileOnDisk(src), null)
    waitFor("sent", 30000) { ph.e.transfers().first { it.id == tid }.state == XState.Done }
    waitFor("on PC") { pc.many("notes $pfp").any { it.contains("file report.bin 6000000") } }
    val got = File(pc.inbox, "report.bin").takeIf { it.exists() } ?: pc.inbox.walkTopDown().first { it.isFile && it.name.startsWith("report") }
    check(got.readBytes().contentEquals(data), "PC has the file intact")
    // files: PC -> phone (the phone user accepts)
    val f2 = File(pc.dir, "from-pc.bin"); f2.writeBytes(data.copyOf(2_500_000))
    check(pc.one("file $pfp ${f2.path}") == "OK started", "file started")
    waitFor("offer on phone") { ph.e.transfers().any { it.incoming && it.state == XState.Offered && it.name == "from-pc.bin" } }
    val inTid = ph.e.transfers().first { it.incoming }.id
    check(ph.e.acceptFile(inTid), "accept")
    waitFor("file arrived", 30000) { ph.received.size == 1 }
    check(ph.received[0].readBytes().contentEquals(data.copyOf(2_500_000)), "phone has the file intact")
    waitFor("pc says done") { pc.many("transfers").any { it.contains("Done") && it.contains("from-pc.bin") } }
    println("file PC->phone ok")

    // --- pictures both ways: the preview that one program makes is accepted by the other
    val png = File(dp, "phone-photo.png"); makePicture(png, 320, 200)
    val pt = ph.e.sendFile(pcfp, PicSource(png, jpegPreview(48, 30)), null)
    waitFor("PC saw the picture offer with preview") { pc.many("transfers").any { it.contains("phone-photo.png") && it.endsWith("thumb=1") } }
    waitFor("picture sent", 30000) { ph.e.transfers().first { it.id == pt }.state == XState.Done }
    val pcPng = File(pc.dir, "pc-photo.png"); makePicture(pcPng, 400, 250)
    check(pc.one("file $pfp ${pcPng.path}") == "OK started", "picture started")
    waitFor("picture offer with the PC's preview on the phone") { ph.e.transfers().any { it.incoming && it.name == "pc-photo.png" && it.thumb != null } }
    val px = ph.e.transfers().first { it.incoming && it.name == "pc-photo.png" }
    check(Thumb.valid(px.thumb!!) && Thumb.dimensions(Thumb.decode(px.thumb!!)!!) == (48 to 30), "PC preview is 48x30: ${Thumb.dimensions(Thumb.decode(px.thumb!!)!!)}")
    check(ph.e.inboxFile(px.id) == null, "not downloaded before the phone user accepts")
    check(ph.e.acceptFile(px.id), "accept")
    waitFor("pc picture arrived", 30000) { ph.e.notes(pcfp).any { it.file?.name == "pc-photo.png" && it.file?.thumb != null } }
    println("pictures ok")

    // --- the PC is away from the phone's point of view / the phone is away from the PC's: text waits on the PC side
    ph.e.stop(); Thread.sleep(300)
    val q = pc.one("text $pfp sent while the phone is off").split(" ")
    check(q[0] == "OK" && q[2] == "true", "queued on PC: $q")
    ph = Phone(dp, "Pixel", ph.port)
    waitFor("delivered after phone is back", 25000) { ph.e.notes(pcfp).any { it.text == "sent while the phone is off" } }
    waitFor("PC no longer pending") { pc.many("notes $pfp").none { it.contains("sent while the phone is off") && it.split(" ")[3] == "true" } }
    println("PC offline queue ok")

    // --- the phone is away: its text waits on the phone and goes out when the PC is back (same PC process, so use a second PC)
    println("unpair from PC")
    check(pc.one("unpair $pfp") == "OK true", "unpair")
    waitFor("phone forgot PC") { ph.e.peers().isEmpty() }
    println("unpair ok")

    // --- the PC asks the phone
    val r = pc.one("pair 127.0.0.1 ${ph.port}")
    check(r.startsWith("PAIR"), r)
    val (pid, pcode) = r.split(" ").let { it[1] to it[2] + " " + it[3] }
    waitFor("phone sees request") { ph.e.pairRequests().isNotEmpty() }
    val inc = ph.e.pairRequests()[0]
    check(inc.code == pcode && inc.name == "Desk PC", "phone request ${inc.code} vs $pcode")
    ph.e.pairAccept(inc.id)
    check(pc.one("confirm $pid") == "OK true", "pc confirm")
    waitFor("paired") { ph.e.peers().size == 1 && pc.many("peers").size == 1 }
    check(ph.e.peers()[0].name == "Desk PC", "name")
    ph.e.sendText(pcfp, "works the other way too")
    waitFor("pc text 2") { pc.many("notes $pfp").any { it.contains("works the other way too") } }
    println("PC->phone pairing ok ($pcode)")
    pc.quit()
    println("INTEROP OK")
    System.exit(0)
}
