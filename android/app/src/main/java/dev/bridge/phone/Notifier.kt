package dev.bridge.phone

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** What the user chose in Settings for notifications about new messages and files. */
data class NotifySettings(val on: Boolean = true, val popup: Boolean = true, val sound: Boolean = true, val blink: Boolean = true)

/**
 * Notifications about new messages and files while Bridge is not on screen. Android fixes sound and pop-up of a
 * notification channel when the channel is created, so there is one channel per combination of the three choices.
 */
object Notifier {
    private lateinit var app: Application
    private lateinit var store: Store
    private val _settings = MutableStateFlow(NotifySettings())
    val settings: StateFlow<NotifySettings> = _settings.asStateFlow()

    fun init(application: Application, s: Store) {
        app = application
        store = s
        _settings.value = s.notifySettings()
    }

    fun change(f: (NotifySettings) -> NotifySettings) {
        val next = f(_settings.value)
        _settings.value = next
        store.saveNotifySettings(next)
    }

    private fun channel(s: NotifySettings): String {
        val id = "messages_" + (if (s.popup) "p" else "-") + (if (s.sound) "s" else "-") + (if (s.blink) "b" else "-")
        val manager = app.getSystemService(android.content.Context.NOTIFICATION_SERVICE) as NotificationManager
        if (manager.getNotificationChannel(id) == null) {
            val parts = listOfNotNull("pop-up".takeIf { s.popup }, "sound".takeIf { s.sound }, "vibration and light".takeIf { s.blink })
            val importance = when {
                s.popup -> NotificationManager.IMPORTANCE_HIGH
                s.sound -> NotificationManager.IMPORTANCE_DEFAULT
                else -> NotificationManager.IMPORTANCE_LOW
            }
            val c = NotificationChannel(id, if (parts.isEmpty()) "Messages (silent)" else "Messages (" + parts.joinToString(", ") + ")", importance)
            if (!s.sound) c.setSound(null, null)
            c.enableVibration(s.blink)
            c.enableLights(s.blink)
            manager.createNotificationChannel(c)
        }
        return id
    }

    /** A new message or file from `from`. The caller decides that Bridge is not on screen. */
    fun message(from: String, text: String) {
        val s = _settings.value
        if (!s.on) return
        try {
            val manager = app.getSystemService(android.content.Context.NOTIFICATION_SERVICE) as NotificationManager
            val open = PendingIntent.getActivity(app, 3, Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
            val shown = text.trim().lineSequence().firstOrNull().orEmpty().let { if (it.length > 120) it.take(119) + "…" else it }
            val n = NotificationCompat.Builder(app, channel(s))
                .setSmallIcon(R.drawable.ic_stat)
                .setContentTitle(from)
                .setContentText(shown)
                .setCategory(NotificationCompat.CATEGORY_MESSAGE)
                .setAutoCancel(true)
                .setContentIntent(open)
                .build()
            manager.notify("msg:$from".hashCode(), n)
        } catch (e: Exception) {
            // notifications may not be allowed
        }
    }
}
