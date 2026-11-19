package biadeploy.android

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.bia_deploy_mobile.BrowserKey
import uniffi.bia_deploy_mobile.browserKey
import uniffi.bia_deploy_mobile.savedBias

/**
 * One bia this phone holds the key of. `host` is what is dialled, a port included; `path` the
 * page of bia's web app to open (/ is the chat, a link from bia names another); `shown` the
 * address the title shows, when not `host`; `name` what a person calls it.
 */
data class ChatTarget(val host: String, val folder: String, val path: String = "/", val shown: String? = null, val name: String = "") {
    private val bracketed get() = host.startsWith("[")
    private val ipv6 get() = !bracketed && host.count { it == ':' } > 1
    val origin get() = if (ipv6) "https://[$host]" else "https://$host"
    val url get() = origin + (if (path.startsWith("/")) path else "/")

    /** bia's page in the browser with this phone's key in the link: the page opens it with the
     *  password the person types and keeps a key of its own. */
    fun link(hash: String) = url + (if ("#" in url) "&" else "#") + "biaconfig=" + hash
}

/** Where the files for browsers go (the FileProvider's `share/`). */
private fun keysDir(context: Context) = File(context.cacheDir, "share").apply { mkdirs() }

private fun copyPassword(context: Context, password: String) {
    val cb = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    cb.setPrimaryClip(ClipData.newPlainText("bia password", password))
}

@Composable
private fun Big(text: String, onClick: () -> Unit) {
    Button(onClick, shape = RoundedCornerShape(22.dp), colors = ButtonDefaults.buttonColors(containerColor = Accent, contentColor = Color.White),
        modifier = Modifier.fillMaxWidth().height(48.dp)) { Text(text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold) }
}

/** bia lives in the browser: this phone hands it the key. The page opens it with a password this
 *  app shows and copies; the browser then keeps a key of its own. */
@Composable
fun ChatScreen(target: ChatTarget, onMoved: (Int) -> Unit = {}, onClose: () -> Unit) {
    val context = LocalContext.current
    var key by remember { mutableStateOf<BrowserKey?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    BackHandler { onClose() }
    LaunchedEffect(target) {
        runCatching { withContext(Dispatchers.IO) { browserKey(target.folder, target.host, target.name, keysDir(context).absolutePath) } }
            .onSuccess { key = it; copyPassword(context, it.password) }
            .onFailure { problem = "This phone cannot read bia's key: ${it.message}" }
    }
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row { TextButton(onClose) { Text("‹  Close") } }
            Text("bia at ${target.shown ?: target.host}", fontWeight = FontWeight.SemiBold)
            Text("bia opens in your browser", fontSize = 22.sp, fontWeight = FontWeight.Bold)
            Text("This phone gives the browser bia's key. On bia's page, paste this password (it is copied already). After that the browser keeps a key of its own and lets you in by itself.",
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            val k = key
            when {
                k != null -> {
                    SelectionContainer { Text(k.password, fontFamily = FontFamily.Monospace, fontSize = 20.sp, modifier = Modifier.padding(vertical = 6.dp)) }
                    TextButton({ copyPassword(context, k.password); Toast.makeText(context, "Password copied", Toast.LENGTH_SHORT).show() }) { Text("Copy password") }
                    Big("Open in browser") {
                        copyPassword(context, k.password)
                        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(target.link(k.hash)))) }
                            .onFailure { problem = "No browser here: ${it.message}" }
                    }
                    Text("Another browser, or this one later: on bia's sign-in page choose the file, pick bia deploy as where it comes from, then this bia. It opens with the same password.",
                        fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                problem == null -> CircularProgressIndicator()
            }
            problem?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }
}

/**
 * The browser's file picker ("choose the file" on bia's sign-in page) offers this app: the person
 * picks a saved bia and the page gets its .biaconfig. The password is copied for pasting there.
 */
class PickKeyActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setResult(Activity.RESULT_CANCELED)
        setContent {
            BiaTheme {
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        Text("Which bia's key?", fontSize = 22.sp, fontWeight = FontWeight.Bold)
                        Text("bia's page gets its .biaconfig; its password is copied for you to paste there.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        val bias = remember { savedBias() }
                        if (bias.isEmpty()) Text("This phone holds no bia's key yet.")
                        bias.forEach { b ->
                            HorizontalDivider()
                            Column(Modifier.fillMaxWidth().clickable { pick(b.folder, b.host, b.name) }.padding(vertical = 10.dp)) {
                                Text(b.name.ifEmpty { b.host }, fontWeight = FontWeight.SemiBold)
                                if (b.name.isNotEmpty()) Text(b.host, fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                        TextButton({ finish() }) { Text("Cancel") }
                    }
                }
            }
        }
    }

    private fun pick(folder: String, host: String, name: String) {
        val k = runCatching { browserKey(folder, host, name, keysDir(this).absolutePath) }.getOrElse {
            Toast.makeText(this, "This phone cannot read bia's key: ${it.message}", Toast.LENGTH_LONG).show()
            return
        }
        copyPassword(this, k.password)
        Toast.makeText(this, "Password copied: paste it on bia's page", Toast.LENGTH_LONG).show()
        val uri = FileProvider.getUriForFile(this, "$packageName.share", File(k.path))
        setResult(Activity.RESULT_OK, Intent().setData(uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
        finish()
    }
}
