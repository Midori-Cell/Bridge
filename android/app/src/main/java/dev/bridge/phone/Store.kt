package dev.bridge.phone

import android.content.Context
import android.os.Build
import android.provider.Settings
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.Json

/** Everything the app remembers, in the app's private storage (no backup, see the manifest). */
class Store(private val context: Context) {
    private val prefs = context.getSharedPreferences("bridge", Context.MODE_PRIVATE)
    private val json = Json { ignoreUnknownKeys = true }
    private val pcList = ListSerializer(Pc.serializer())
    private val strings = MapSerializer(String.serializer(), String.serializer())

    fun pcs(): List<Pc> = prefs.getString("pcs", null)?.let { runCatching { json.decodeFromString(pcList, it) }.getOrNull() } ?: emptyList()

    fun savePcs(list: List<Pc>) {
        prefs.edit().putString("pcs", json.encodeToString(pcList, list)).apply()
    }

    var activeId: String?
        get() = prefs.getString("active", null)
        set(v) = prefs.edit().putString("active", v).apply()

    /** The name this phone shows on a PC. Defaults to the name the user gave the phone in Android. */
    var phoneName: String
        get() = prefs.getString("phone_name", null) ?: defaultPhoneName()
        set(v) = prefs.edit().putString("phone_name", v.trim()).apply()

    private fun defaultPhoneName(): String {
        val given = runCatching { Settings.Global.getString(context.contentResolver, "device_name") }.getOrNull()
        return (given?.takeIf { it.isNotBlank() } ?: Build.MODEL ?: "Phone").take(40)
    }

    /** Highest note id the user has seen on this PC; newer files are downloaded automatically. */
    fun seen(pcId: String): Long? = if (prefs.contains("seen_$pcId")) prefs.getLong("seen_$pcId", 0) else null

    fun setSeen(pcId: String, id: Long) {
        prefs.edit().putLong("seen_$pcId", id).apply()
    }

    /** File id -> where it was saved (a content:// address in Downloads/Bridge). */
    fun saved(): Map<String, String> = prefs.getString("saved", null)?.let { runCatching { json.decodeFromString(strings, it) }.getOrNull() } ?: emptyMap()

    fun saveSaved(map: Map<String, String>) {
        prefs.edit().putString("saved", json.encodeToString(strings, map)).apply()
    }

    var themeMode: ThemeMode
        get() = runCatching { ThemeMode.valueOf(prefs.getString("theme", "") ?: "") }.getOrDefault(ThemeMode.System)
        set(v) = prefs.edit().putString("theme", v.name).apply()

    /** Times in the chat as 21:14 (true) or 9:14 PM (false). Until the user chooses, the phone's own clock setting. */
    var time24: Boolean
        get() = if (prefs.contains("time24")) prefs.getBoolean("time24", true) else android.text.format.DateFormat.is24HourFormat(context)
        set(v) = prefs.edit().putBoolean("time24", v).apply()

    private val queuedList = ListSerializer(Queued.serializer())

    /** Texts that wait for a PC that was not reachable, oldest first. */
    fun outbox(): List<Queued> = prefs.getString("outbox", null)?.let { runCatching { json.decodeFromString(queuedList, it) }.getOrNull() } ?: emptyList()

    fun saveOutbox(list: List<Queued>) {
        prefs.edit().putString("outbox", json.encodeToString(queuedList, list)).apply()
    }

    /** Messages with this id or lower are hidden in the chat with this PC ("Clear chat" on this phone). */
    fun clearedUpTo(pcId: String): Long = prefs.getLong("cleared_$pcId", 0L)

    fun setClearedUpTo(pcId: String, id: Long) {
        prefs.edit().putLong("cleared_$pcId", id).apply()
    }

    var askedNotifications: Boolean
        get() = prefs.getBoolean("asked_notifications", false)
        set(v) = prefs.edit().putBoolean("asked_notifications", v).apply()
}
