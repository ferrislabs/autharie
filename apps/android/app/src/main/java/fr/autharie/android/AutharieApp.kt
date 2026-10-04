package fr.autharie.android

import android.app.Application
import dagger.hilt.android.HiltAndroidApp
import fr.autharie.android.notifications.NotificationChannels

@HiltAndroidApp
class AutharieApp : Application() {
    override fun onCreate() {
        super.onCreate()
        NotificationChannels.create(this)
    }
}
