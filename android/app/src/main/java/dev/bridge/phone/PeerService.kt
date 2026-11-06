package dev.bridge.phone

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Keeps Bridge able to receive from other phones while the app is closed. It only holds the notification and keeps the
 * process alive; the connections themselves are handled by [Phones]. It runs only when the user turned on
 * "Receive while app is closed" in Settings.
 */
class PeerService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Receiving", NotificationManager.IMPORTANCE_MIN))
        val open = PendingIntent.getActivity(this, 2, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        val notification: Notification = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("Bridge")
            .setContentText("Ready to receive from your other devices")
            .setOngoing(true)
            .setContentIntent(open)
            .build()
        ServiceCompat.startForeground(this, ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        Phones.startForService()
        return START_STICKY
    }

    companion object {
        private const val CHANNEL = "receiving"
        private const val ID = 2

        fun start(context: Context) {
            try {
                ContextCompat.startForegroundService(context, Intent(context, PeerService::class.java))
            } catch (e: Exception) {
                // the app may not start a service from the background right now; it starts with the app next time
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, PeerService::class.java))
        }
    }
}
