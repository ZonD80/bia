package biadeploy.android

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.runtime.LaunchedEffect

class MainActivity : ComponentActivity() {
    private val model: DeployModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // store screenshots: made-up data, no network (debug builds only)
        if (BuildConfig.DEBUG && savedInstanceState == null) intent.getStringExtra("demo")?.let { model.demo(it, filesDir) }
        // a saved bia's chat at once, for testing (debug builds only)
        if (BuildConfig.DEBUG && savedInstanceState == null) intent.getStringExtra("chat")?.let { host ->
            model.saved.firstOrNull { it.host == host }?.let { model.chat = ChatTarget(it.host, it.folder) }
        }
        if (savedInstanceState == null) openedFile(intent)
        setContent {
            // the takeover and the wait for bia take minutes: keep the screen (and this app) on
            val running = model.step == Step.Running || (model.step == Step.Installed && model.up == null)
            LaunchedEffect(running) {
                if (running) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
            BiaTheme { App(model) }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        openedFile(intent)
    }

    /** A .biaconfig opened from elsewhere: a private copy, then its password is asked for. */
    private fun openedFile(intent: Intent?) {
        if (intent?.action != Intent.ACTION_VIEW) return
        val uri = intent.data ?: return
        model.offerConfig(cacheDir) { dest -> contentResolver.openInputStream(uri)?.use { it.copyLimited(dest) } }
    }
}
