package dev.bridge.phone

import android.app.Application

class BridgeApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Bridge.init(this)
    }
}
