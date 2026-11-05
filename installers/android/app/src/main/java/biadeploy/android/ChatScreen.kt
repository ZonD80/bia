package biadeploy.android

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.net.http.SslCertificate
import android.net.http.SslError
import android.util.Base64
import android.view.ViewGroup
import android.webkit.ClientCertRequest
import android.webkit.JavascriptInterface
import android.webkit.PermissionRequest
import android.webkit.SslErrorHandler
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import java.io.ByteArrayInputStream
import java.io.File
import java.security.KeyStore
import java.security.PrivateKey
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import uniffi.bia_deploy_mobile.p12Password

/** One bia this phone holds the key of. `host` is what is dialled, a port included. */
data class ChatTarget(val host: String, val folder: String) {
    private val bracketed get() = host.startsWith("[")
    private val ipv6 get() = !bracketed && host.count { it == ':' } > 1
    /** The address without a port, as requests report it. */
    val name get() = if (bracketed) host.substringAfter('[').substringBefore(']') else if (ipv6) host else host.substringBefore(':')
    val url get() = if (ipv6) "https://[$host]/" else "https://$host/"
    fun isHere(h: String?) = h?.trim('[', ']') == name
}

/** bia's own CA and root's certificate, as kept in its folder on this phone. */
private class ChatKeys(folder: String) {
    val ca: X509Certificate = File(folder, "ca.crt").inputStream().use {
        CertificateFactory.getInstance("X.509").generateCertificate(it) as X509Certificate
    }
    val key: PrivateKey
    val chain: Array<X509Certificate>

    init {
        val store = KeyStore.getInstance("PKCS12")
        val password = p12Password().toCharArray()
        File(folder, "bia-root.p12").inputStream().use { store.load(it, password) }
        val alias = store.aliases().toList().first { store.isKeyEntry(it) }
        key = store.getKey(alias, password) as PrivateKey
        chain = store.getCertificateChain(alias).map { it as X509Certificate }.toTypedArray()
    }

    /** A server certificate this bia's CA signed: bia's CA is in no public store, and only for it. */
    fun signedByCa(cert: SslCertificate): Boolean = runCatching {
        val der = SslCertificate.saveState(cert).getByteArray("x509-certificate")!!
        val x = CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(der)) as X509Certificate
        x.checkValidity()
        x.verify(ca.publicKey)
    }.isSuccess
}

/** Files on bia's page are blobs, which the system's download manager cannot fetch: the page hands them over. */
private const val SAVE_BLOBS = """
(function () {
  if (window.__biaSaves) return; window.__biaSaves = true;
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest && e.target.closest('a[download]');
    if (!a || !a.href.startsWith('blob:')) return;
    e.preventDefault(); e.stopPropagation();
    fetch(a.href).then(function (r) { return r.blob(); }).then(function (b) {
      var f = new FileReader();
      f.onload = function () { BiaApp.save(a.download || 'file', f.result.split(',')[1]); };
      f.readAsDataURL(b);
    });
  }, true);
})();
"""

/** bia's web app inside this app: root's key and bia's CA come from this phone, nothing to install. */
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun ChatScreen(target: ChatTarget, onClose: () -> Unit) {
    val context = LocalContext.current
    val keys = remember(target) { runCatching { ChatKeys(target.folder) } }
    val failed = keys.exceptionOrNull()
    var web by remember { mutableStateOf<WebView?>(null) }
    var problem by remember { mutableStateOf(failed?.let { "This phone cannot read bia's key: ${it.message}" }) }

    var fileCallback by remember { mutableStateOf<ValueCallback<Array<Uri>>?>(null) }
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        fileCallback?.onReceiveValue(uris.toTypedArray())
        fileCallback = null
    }
    var micRequest by remember { mutableStateOf<PermissionRequest?>(null) }
    val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        micRequest?.let { if (ok) it.grant(arrayOf(PermissionRequest.RESOURCE_AUDIO_CAPTURE)) else it.deny() }
        micRequest = null
    }
    var pending by remember { mutableStateOf<ByteArray?>(null) }
    val saveFile = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("*/*")) { uri ->
        val data = pending
        pending = null
        if (uri != null && data != null) runCatching { context.contentResolver.openOutputStream(uri)!!.use { it.write(data) } }
    }

    BackHandler {
        val w = web
        if (w != null && w.canGoBack()) w.goBack() else onClose()
    }
    DisposableEffect(target) {
        onDispose { web?.destroy() }
    }

    val background = MaterialTheme.colorScheme.background
    // its own background: the status bar's icons suit the app's colors, whatever the page is doing
    Surface(Modifier.fillMaxSize(), color = background) {
    Column(Modifier.fillMaxSize().safeDrawingPadding().imePadding()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClose) { Text("‹  Close") }
            Text("bia at ${target.host}", Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
            TextButton({ problem = null; web?.reload() }) { Text("Reload") }
        }
        problem?.let { Text(it, Modifier.fillMaxWidth().padding(16.dp)) }
        val k = keys.getOrNull() ?: return@Column
        AndroidView(
            modifier = Modifier.fillMaxSize(),
            factory = { ctx ->
                WebView(ctx).apply {
                    layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                    setBackgroundColor(background.toArgb())
                    settings.javaScriptEnabled = true
                    settings.domStorageEnabled = true
                    settings.mediaPlaybackRequiresUserGesture = false
                    settings.allowFileAccess = false
                    settings.allowContentAccess = true
                    WebView.setWebContentsDebuggingEnabled(BuildConfig.DEBUG)
                    addJavascriptInterface(object {
                        @JavascriptInterface
                        fun save(name: String, base64: String) {
                            post {
                                pending = Base64.decode(base64, Base64.DEFAULT)
                                saveFile.launch(name)
                            }
                        }
                    }, "BiaApp")
                    webViewClient = chatClient(target, k, ctx, onProblem = { problem = it })
                    webChromeClient = object : WebChromeClient() {
                        override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<Uri>>, params: FileChooserParams): Boolean {
                            fileCallback?.onReceiveValue(null)
                            fileCallback = callback
                            pickFiles.launch(arrayOf("*/*"))
                            return true
                        }

                        override fun onPermissionRequest(request: PermissionRequest) {
                            val ours = target.isHere(request.origin.host)
                            if (!ours || PermissionRequest.RESOURCE_AUDIO_CAPTURE !in request.resources) {
                                request.deny()
                            } else if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                                request.grant(arrayOf(PermissionRequest.RESOURCE_AUDIO_CAPTURE))
                            } else {
                                micRequest = request
                                askMic.launch(Manifest.permission.RECORD_AUDIO)
                            }
                        }
                    }
                    web = this
                    loadUrl(target.url)
                }
            },
        )
    }
    }
}

private fun chatClient(target: ChatTarget, keys: ChatKeys, context: Context, onProblem: (String) -> Unit) = object : WebViewClient() {
    override fun onReceivedClientCertRequest(view: WebView, request: ClientCertRequest) {
        if (target.isHere(request.host)) request.proceed(keys.key, keys.chain) else request.cancel()
    }

    override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) {
        // Only "not a public CA", only this bia's address, and only what this bia's CA signed.
        val otherFault = listOf(SslError.SSL_IDMISMATCH, SslError.SSL_DATE_INVALID, SslError.SSL_EXPIRED, SslError.SSL_NOTYETVALID, SslError.SSL_INVALID).any { error.hasError(it) }
        val here = target.isHere(runCatching { Uri.parse(error.url).host }.getOrNull())
        if (here && !otherFault && keys.signedByCa(error.certificate)) {
            handler.proceed()
        } else {
            handler.cancel()
            onProblem("bia's certificate does not match the key on this phone, so the chat stayed closed.")
        }
    }

    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
        if (target.isHere(request.url.host)) return false
        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, request.url)) }
        return true
    }

    override fun onPageFinished(view: WebView, url: String?) {
        view.evaluateJavascript(SAVE_BLOBS, null)
    }

    override fun onReceivedError(view: WebView, request: WebResourceRequest, error: android.webkit.WebResourceError) {
        if (request.isForMainFrame) onProblem("bia did not answer at ${target.host}: ${error.description}. Reload tries again.")
    }
}
