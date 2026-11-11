package dev.bridge.phone

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/**
 * This phone's own copy of each conversation, kept in a file per PC (or per phone) in the app's private storage.
 * The phone does not depend on the other side for its history: when the PC clears its chat, the phone still has
 * everything it has already fetched, and "Clear chat" on this phone deletes the texts here for good.
 *
 * No Android classes are used, so the logic can be tested on a plain JVM.
 */
class History(private val dir: File, private val limit: Int = 5000) {
    /** What is known about one conversation. `last` and `del` are the cursors to ask the PC for what is new. */
    class Conv(var notes: List<Note> = emptyList(), var last: Long? = null, var del: Long = 0)

    private val cache = HashMap<String, Conv>()

    private fun file(id: String) = File(dir, id.filter { it.isLetterOrDigit() || it == '-' || it == '_' }.take(80) + ".json")

    @Synchronized
    private fun conv(id: String): Conv = cache.getOrPut(id) { read(file(id)) }

    @Synchronized
    fun notes(id: String): List<Note> = conv(id).notes

    /** The newest note id fetched so far (null: this conversation was never fetched) and the deletion counter. */
    @Synchronized
    fun cursor(id: String): Pair<Long?, Long> = conv(id).let { it.last to it.del }

    /**
     * Store what the other side sent: new notes (a note with a known id or message id replaces the old one), the
     * messages it says were deleted for both, and the cursors for the next request.
     */
    @Synchronized
    fun merge(id: String, news: List<Note>, gone: Set<String>, last: Long?, del: Long?): List<Note> {
        val c = conv(id)
        var list = c.notes
        if (news.isNotEmpty()) {
            val ids = news.map { it.id }.toSet()
            val mids = news.mapNotNull { it.mid.ifEmpty { null } }.toSet()
            list = list.filter { it.id !in ids && (it.mid.isEmpty() || it.mid !in mids) } + news.filter { !it.pending }
        }
        if (gone.isNotEmpty()) list = list.filter { it.mid.isEmpty() || it.mid !in gone }
        list = list.sortedBy { it.id }
        if (list.size > limit) list = list.takeLast(limit)
        val changed = list != c.notes || (last != null && last != c.last) || (del != null && del != c.del)
        c.notes = list
        if (last != null) c.last = maxOf(last, c.last ?: 0)
        if (del != null) c.del = del
        if (changed) write(file(id), c)
        return list
    }

    /** One message that was delivered just now (the answer to a send). */
    fun add(id: String, note: Note): List<Note> = merge(id, listOf(note), emptySet(), null, null)

    /** The user deleted one message. */
    @Synchronized
    fun remove(id: String, noteId: Long): List<Note> {
        val c = conv(id)
        c.notes = c.notes.filter { it.id != noteId }
        write(file(id), c)
        return c.notes
    }

    /** "Clear chat": the texts up to `upTo` are deleted from this phone. Files stay (the list of files shows them). */
    @Synchronized
    fun clearTexts(id: String, upTo: Long): List<Note> {
        val c = conv(id)
        c.notes = c.notes.filter { it.id > upTo || it.isFile }
        write(file(id), c)
        return c.notes
    }

    @Synchronized
    fun forget(id: String) {
        cache.remove(id)
        file(id).delete()
    }

    // ------------------------------------------------------------------------------------ files

    private fun read(f: File): Conv {
        return try {
            val o = Json.parseToJsonElement(f.readText()).jsonObject
            Conv(
                o["notes"]?.jsonArray?.mapNotNull { runCatching { note(it.jsonObject) }.getOrNull() } ?: emptyList(),
                o["last"]?.takeIf { it !is JsonNull }?.let { (it as JsonPrimitive).longOrNull },
                (o["del"] as? JsonPrimitive)?.longOrNull ?: 0,
            )
        } catch (e: Exception) {
            Conv()
        }
    }

    private fun write(f: File, c: Conv) {
        try {
            f.parentFile?.mkdirs()
            val o = buildJsonObject {
                put("v", 1)
                c.last?.let { put("last", it) }
                put("del", c.del)
                put("notes", JsonArray(c.notes.map { json(it) }))
            }
            val tmp = File(f.path + ".tmp")
            tmp.writeText(o.toString())
            if (!tmp.renameTo(f)) {
                f.delete()
                tmp.renameTo(f)
            }
        } catch (e: Exception) {
            // the history is a convenience; a full disk must not stop the app
        }
    }

    companion object {
        fun json(n: Note): JsonObject = buildJsonObject {
            put("id", n.id)
            put("mid", n.mid)
            put("ts", n.ts)
            put("src", n.src)
            put("kind", n.kind)
            if (n.text != null) put("text", n.text)
            n.file?.let { f ->
                put("file", buildJsonObject {
                    put("id", f.id)
                    put("name", f.name)
                    put("size", f.size)
                    put("sha256", f.sha256)
                    put("dir", f.dir)
                    if (f.thumb != null) put("thumb", f.thumb)
                })
            }
            n.reply?.let { r ->
                put("reply", buildJsonObject {
                    put("mid", r.mid)
                    put("src", r.src)
                    put("preview", r.preview)
                })
            }
        }

        fun note(o: JsonObject): Note {
            fun s(k: String, from: JsonObject = o) = (from[k] as? JsonPrimitive)?.contentOrNull ?: ""
            fun l(k: String, from: JsonObject = o) = (from[k] as? JsonPrimitive)?.longOrNull ?: 0L
            return Note(
                id = l("id"),
                mid = s("mid"),
                ts = (o["ts"] as? JsonPrimitive)?.doubleOrNull ?: 0.0,
                src = s("src"),
                kind = s("kind").ifEmpty { "text" },
                text = (o["text"] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.contentOrNull,
                file = (o["file"] as? JsonObject)?.let { f -> FileRef(s("id", f), s("name", f), l("size", f), s("sha256", f), s("dir", f), (f["thumb"] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.contentOrNull) },
                reply = (o["reply"] as? JsonObject)?.let { r -> Reply(s("mid", r), s("src", r), s("preview", r)) },
            )
        }
    }
}
