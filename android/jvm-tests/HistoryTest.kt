import dev.bridge.phone.*
import java.io.File

fun check(c: Boolean, m: String) { if (!c) throw AssertionError(m) }
fun n(id: Long, mid: String, text: String?, file: FileRef? = null) = Note(id = id, mid = mid, ts = id.toDouble(), src = "pc", kind = if (file != null) "file" else "text", text = text, file = file, reply = if (id == 3L) Reply("a","pc","hi") else null)

fun main() {
    val d = File.createTempFile("hist", "").also { it.delete(); it.mkdirs() }
    var h = History(d)
    check(h.cursor("pc1") == (null to 0L), "fresh")
    h.merge("pc1", listOf(n(1,"m1","one"), n(2,"m2","two"), n(3,"m3",null, FileRef("f1","a.txt",5,"abc","to_phone"))), emptySet(), 3, 0)
    check(h.notes("pc1").size == 3, "3 notes")
    // restart: everything is still there, cursor too
    h = History(d)
    check(h.notes("pc1").map { it.id } == listOf(1L,2L,3L), "reloaded")
    check(h.cursor("pc1") == (3L to 0L), "cursor reloaded ${h.cursor("pc1")}")
    check(h.notes("pc1")[2].file?.name == "a.txt" && h.notes("pc1")[2].reply?.preview == "hi", "fields")
    // the PC cleared: nothing arrives, the phone keeps all
    h.merge("pc1", emptyList(), emptySet(), 3, 5)
    check(h.notes("pc1").size == 3 && h.cursor("pc1").second == 5L, "keeps after PC clear")
    // delete for both
    h.merge("pc1", emptyList(), setOf("m2"), null, null)
    check(h.notes("pc1").map { it.mid } == listOf("m1","m3"), "deleted")
    // same message again replaces
    h.merge("pc1", listOf(n(1,"m1","one!")), emptySet(), null, null)
    check(h.notes("pc1").first().text == "one!" && h.notes("pc1").size == 2, "replace")
    // clear chat on the phone: texts go, files stay
    h.clearTexts("pc1", 3)
    check(h.notes("pc1").map { it.id } == listOf(3L), "clear texts")
    // cursor never goes backwards
    h.merge("pc1", emptyList(), emptySet(), 1, null)
    check(h.cursor("pc1").first == 3L, "monotonic")
    // other conversation untouched, forget works
    check(h.notes("pc2").isEmpty(), "other")
    h.forget("pc1")
    check(History(d).notes("pc1").isEmpty() && History(d).cursor("pc1").first == null, "forgotten")
    // damaged file
    File(d, "bad.json").writeText("{not json")
    check(History(d).notes("bad").isEmpty(), "bad file")
    // limit
    val small = History(d, 3)
    small.merge("x", (1L..10L).map { n(it, "m$it", "t$it") }, emptySet(), 10, 0)
    check(small.notes("x").map { it.id } == listOf(8L,9L,10L), "limit")
    println("history ok")
}
