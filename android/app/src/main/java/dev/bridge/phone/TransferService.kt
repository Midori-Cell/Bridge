package dev.bridge.phone

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/**
 * Keeps the process alive and shows a notification while files are on their way, so a transfer goes on
 * after the screen is closed. The work itself is done by [Bridge]; this service only holds the notification.
 * It also carries the files' read permission: a file shared from another app can only be read while this service runs.
 */
class TransferService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var watching = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.channel_transfers), NotificationManager.IMPORTANCE_LOW))
        ServiceCompat.startForeground(this, ID, build("Bridge", "Starting...", null), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)

        if (intent?.action == ACTION_SEND) {
            val clip = intent.clipData
            val uris = if (clip == null) emptyList() else (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
            val target = intent.getStringExtra(EXTRA_PC)
            val phone = Phones.peers.value.firstOrNull { it.fp == target }
            val pc = Bridge.pcs.value.firstOrNull { it.id == target } ?: Bridge.active.value
            if (uris.isNotEmpty()) {
                if (phone != null) Phones.sendFiles(uris, phone) else if (pc != null) Bridge.sendFiles(uris, pc)
            }
        }
        if (!watching) {
            watching = true
            scope.launch {
                combine(Bridge.transfers, Phones.transfers) { a, b -> a + b }.collectLatest { list ->
                    val busy = list.filter { it.running }
                    if (busy.isEmpty()) {
                        delay(2000) // a new transfer may follow right away
                        stopForeground(STOP_FOREGROUND_REMOVE)
                        stopSelf()
                    } else {
                        val up = busy.count { it.upload }
                        val down = busy.size - up
                        val what = listOfNotNull(
                            if (up > 0) "Sending $up ${if (up == 1) "file" else "files"}" else null,
                            if (down > 0) "Receiving $down ${if (down == 1) "file" else "files"}" else null,
                        ).joinToString(" and ")
                        val total = busy.sumOf { it.size }
                        val percent = if (total > 0) (busy.sumOf { it.done } * 100 / total).toInt().coerceIn(0, 100) else null
                        manager.notify(ID, build("Bridge", what, percent))
                    }
                }
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun build(title: String, text: String, percent: Int?): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        val builder = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle(title)
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(open)
        if (percent != null) builder.setProgress(100, percent, false)
        return builder.build()
    }

    companion object {
        private const val CHANNEL = "transfers"
        private const val ID = 1
        private const val ACTION_SEND = "dev.bridge.phone.SEND_FILES"
        private const val EXTRA_PC = "pc"

        /** Send files to a PC. They are handed over through the service so their read permission outlives the screen that got them. */
        fun sendFiles(context: Context, uris: List<Uri>, pc: Pc) = sendFilesTo(context, uris, pc.id)

        /** The same for a paired phone (`id` is its fingerprint). */
        fun sendFilesTo(context: Context, uris: List<Uri>, id: String) {
            if (uris.isEmpty()) return
            val clip = ClipData.newRawUri("files", uris.first())
            uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) }
            val intent = Intent(context, TransferService::class.java)
                .setAction(ACTION_SEND)
                .putExtra(EXTRA_PC, id)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            intent.clipData = clip
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
