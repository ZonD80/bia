package biadeploy.android

import android.app.Application
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import uniffi.bia_deploy_mobile.BrowserOpener
import uniffi.bia_deploy_mobile.init
import uniffi.bia_deploy_mobile.setBrowserOpener

/** Starts the Rust core: its folder (keys, known servers, bia's keys) and how it opens pages. */
class BiaApp : Application() {
    override fun onCreate() {
        super.onCreate()
        init(filesDir.absolutePath)
        // The ChatGPT sign-in page talks to the core's listener on 127.0.0.1: a Custom Tab
        // keeps this app's process (and that listener) running while the person signs in.
        setBrowserOpener(object : BrowserOpener {
            override fun openUrl(url: String): Boolean = runCatching {
                val tab = CustomTabsIntent.Builder().setShowTitle(true).build()
                tab.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                tab.launchUrl(this@BiaApp, Uri.parse(url))
                true
            }.getOrDefault(false)
        })
    }
}
