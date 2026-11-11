package dev.bridge.phone

import android.app.Application
import android.content.Context
import android.os.Build
import android.provider.Settings
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.Json

/** Everything the app remembers, in the app's private storage (no backup, see the manifest). */
class Store(private val context: Application) {
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

    /** Ids of files the user said no to: they cannot be fetched from the chat afterwards. */
    fun declined(): Set<String> = prefs.getStringSet("declined", null)?.toSet() ?: emptySet()

    fun saveDeclined(ids: Set<String>) {
        prefs.edit().putStringSet("declined", ids).apply()
    }

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

    // ---- phone to phone

    /** Lets other phones on the Wi-Fi find this one while pairing. */
    var visibleToPhones: Boolean
        get() = prefs.getBoolean("visible_to_phones", true)
        set(v) = prefs.edit().putBoolean("visible_to_phones", v).apply()

    /** Keep Bridge able to receive from other phones when the app is closed (a small notification stays). */
    var receiveInBackground: Boolean
        get() = prefs.getBoolean("receive_in_background", false)
        set(v) = prefs.edit().putBoolean("receive_in_background", v).apply()

    /** The port this phone listens on for other phones; the same one each time so paired phones find it again. */
    var p2pPort: Int
        get() = prefs.getInt("p2p_port", Phones.DEFAULT_PORT)
        set(v) = prefs.edit().putInt("p2p_port", v).apply()

    /** The phone whose chat is open (fingerprint), or null while a PC's chat is open. */
    var activePhone: String?
        get() = prefs.getString("active_phone", null)
        set(v) = prefs.edit().putString("active_phone", v).apply()

    /** Per phone: messages up to this id are hidden in the chat ("Clear chat" on this phone). */
    fun phoneCleared(): Map<String, Long> =
        prefs.all.filterKeys { it.startsWith("pcleared_") }.mapNotNull { (k, v) -> (v as? Long)?.let { k.removePrefix("pcleared_") to it } }.toMap()

    fun setPhoneCleared(fp: String, id: Long) {
        prefs.edit().apply { if (id <= 0) remove("pcleared_$fp") else putLong("pcleared_$fp", id) }.apply()
    }

    fun notifySettings() = NotifySettings(
        on = prefs.getBoolean("notify_on", true),
        popup = prefs.getBoolean("notify_popup", true),
        sound = prefs.getBoolean("notify_sound", true),
        blink = prefs.getBoolean("notify_blink", true),
    )

    fun saveNotifySettings(s: NotifySettings) {
        prefs.edit().putBoolean("notify_on", s.on).putBoolean("notify_popup", s.popup).putBoolean("notify_sound", s.sound).putBoolean("notify_blink", s.blink).apply()
    }

    var askedNotifications: Boolean
        get() = prefs.getBoolean("asked_notifications", false)
        set(v) = prefs.edit().putBoolean("asked_notifications", v).apply()
}
