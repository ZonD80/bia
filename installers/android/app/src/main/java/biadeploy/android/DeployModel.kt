package biadeploy.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.bia_deploy_mobile.Brain
import uniffi.bia_deploy_mobile.CloudModel
import uniffi.bia_deploy_mobile.DeployException
import uniffi.bia_deploy_mobile.Done
import uniffi.bia_deploy_mobile.Facts
import uniffi.bia_deploy_mobile.Login
import uniffi.bia_deploy_mobile.Plan
import uniffi.bia_deploy_mobile.RunListener
import uniffi.bia_deploy_mobile.Server
import uniffi.bia_deploy_mobile.SshKey
import uniffi.bia_deploy_mobile.Target
import uniffi.bia_deploy_mobile.WaitListener
import uniffi.bia_deploy_mobile.chatgptSignIn
import uniffi.bia_deploy_mobile.defaultModel
import uniffi.bia_deploy_mobile.identifyKey
import uniffi.bia_deploy_mobile.landSecret
import uniffi.bia_deploy_mobile.newKey
import uniffi.bia_deploy_mobile.readKey
import uniffi.bia_deploy_mobile.SavedBia
import uniffi.bia_deploy_mobile.savedBias
import uniffi.bia_deploy_mobile.savedKeys
import uniffi.bia_deploy_mobile.waitUp
import uniffi.bia_deploy_mobile.wake

enum class Page { Brain, Server, Keys }
enum class Step { Login, Connecting, HostKey, Checking, Confirm, Running, Installed }
enum class How { Password, Key, NewKey }

/** The wizard: bia's brain, then the server. Every core call runs off the main thread. */
class DeployModel : ViewModel() {
    var page by mutableStateOf(Page.Brain)

    // ---- brain ----
    var provider by mutableStateOf("")
    var apiKey by mutableStateOf("")
    var baseUrl by mutableStateOf("")
    var model by mutableStateOf("")
    var ctx by mutableStateOf(131_072u)
    var chatgptJson by mutableStateOf<String?>(null)
    var chatgptEmail by mutableStateOf<String?>(null)
    val models = mutableStateListOf<CloudModel>()
    var lang by mutableStateOf(java.util.Locale.getDefault().language.ifEmpty { "en" })
    var busy by mutableStateOf<String?>(null)
    var error by mutableStateOf<String?>(null)
    var hello by mutableStateOf<String?>(null)
    private var wokeFor by mutableStateOf<String?>(null)
    private var keyChecked = ""

    private fun brainSig() = listOf(provider, apiKey, model, baseUrl, chatgptEmail ?: "").joinToString("|")
    val brainOk get() = wokeFor == brainSig()

    /** The demo scenes (debug builds): the brain counts as woken. */
    internal fun markWoken() { wokeFor = brainSig() }

    fun brain() = Brain(provider, apiKey, model, baseUrl, ctx, chatgptJson)

    private fun work(label: String, block: suspend () -> Unit) {
        if (busy != null) return
        busy = label
        error = null
        viewModelScope.launch {
            try {
                block()
            } catch (e: DeployException) {
                error = when (e) {
                    is DeployException.Failed -> e.reason
                    is DeployException.HostKeyChanged -> "this server's identity changed"
                }
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            } finally {
                busy = null
            }
        }
    }

    fun keyChanged(k: String) {
        apiKey = k
        val key = k.trim()
        if (key.length < 20 || key == keyChecked) return
        keyChecked = key
        work("checking the key…") {
            val info = withContext(Dispatchers.IO) { identifyKey(key) }
            provider = info.provider
            baseUrl = info.baseUrl
            models.clear(); models.addAll(info.models)
            if (model.isBlank()) model = defaultModel(info.provider)
        }
    }

    fun signInChatGpt() = work("finish signing in on the ChatGPT page…") {
        val s = withContext(Dispatchers.IO) { chatgptSignIn() }
        provider = "chatgpt"
        chatgptJson = s.chatgptJson
        chatgptEmail = s.email
        apiKey = ""
        models.clear(); models.addAll(s.models)
        model = s.defaultModel ?: s.models.firstOrNull()?.id ?: ""
        s.models.firstOrNull { it.id == model }?.ctx?.takeIf { it > 0u }?.let { ctx = it }
    }

    fun signOutChatGpt() {
        chatgptJson = null; chatgptEmail = null; provider = ""; model = ""; models.clear()
    }

    fun wakeBia() = work("waking bia…") {
        hello = null
        val g = withContext(Dispatchers.IO) { wake(brain()) }
        g.chatgptJson?.let { chatgptJson = it }
        hello = g.hello
        wokeFor = brainSig()
    }

    // ---- server ----
    var land by mutableStateOf(false)
    var secret by mutableStateOf(landSecret())
    var host by mutableStateOf("")
    var user by mutableStateOf("root")
    var port by mutableStateOf("22")
    var how by mutableStateOf(How.Password)
    var password by mutableStateOf("")
    var privateKey by mutableStateOf("")
    var passphrase by mutableStateOf("")
    var generated by mutableStateOf<SshKey?>(savedKeys().firstOrNull())
    var step by mutableStateOf(Step.Login)
    var facts by mutableStateOf<Facts?>(null)
    var plan by mutableStateOf<Plan?>(null)
    var changedKey by mutableStateOf<Pair<String, String>?>(null)
    var progress by mutableStateOf("")
    var fraction by mutableStateOf<Float?>(null)
    var keySaved by mutableStateOf(false)
    var rebooting by mutableStateOf(false)
    var done by mutableStateOf<Done?>(null)
    var waited by mutableStateOf(0L)
    var up by mutableStateOf<Boolean?>(null)
    var upError by mutableStateOf<String?>(null)
    val log = mutableStateListOf<String>()
    private var server: Server? = null

    fun cleanHost() = host.trim().removePrefix("ssh://").substringAfter('@').trim('[', ']')

    private fun login(): Login {
        val u = host.trim().substringBefore('@', "").ifEmpty { user.trim().ifEmpty { "root" } }
        val key = when (how) {
            How.Key -> privateKey
            How.NewKey -> generated?.let { readKey(it.path) } ?: ""
            How.Password -> ""
        }
        return Login(
            host = cleanHost(),
            port = port.trim().toUShortOrNull() ?: 22u,
            user = u,
            password = password.takeIf { how == How.Password },
            privateKey = key.takeIf { it.isNotBlank() },
            passphrase = passphrase.takeIf { it.isNotEmpty() },
        )
    }

    private fun target() = Target(
        host = cleanHost(),
        brain = if (land) null else brain(),
        lang = lang,
        landSecret = if (land) secret.trim() else null,
    )

    private val listener = object : RunListener {
        override fun step(text: String, fraction: Float?, detail: String) {
            viewModelScope.launch {
                progress = if (detail.isEmpty()) text else "$text — $detail"
                this@DeployModel.fraction = fraction
            }
        }
        override fun line(text: String) {
            viewModelScope.launch { log.add(text); if (log.size > 300) log.removeAt(0) }
        }
        override fun certificate(path: String) { viewModelScope.launch { keySaved = true } }
        override fun rebooting() { viewModelScope.launch { rebooting = true } }
    }

    fun makeKey() = work("making a key…") {
        generated = withContext(Dispatchers.IO) { newKey("bia-deploy") }
    }

    fun connect(accept: String? = null) {
        if (busy != null) return
        error = null
        changedKey = null
        step = Step.Connecting
        viewModelScope.launch {
            try {
                val s = server ?: Server().also { server = it }
                facts = withContext(Dispatchers.IO) { s.connect(login(), accept) }
                step = Step.Checking
                plan = withContext(Dispatchers.IO) { s.plan(target(), listener) }
                step = Step.Confirm
            } catch (e: DeployException.HostKeyChanged) {
                changedKey = e.known to e.now
                step = Step.HostKey
            } catch (e: Exception) {
                error = (e as? DeployException.Failed)?.reason ?: e.message ?: e.toString()
                step = Step.Login
            }
        }
    }

    fun back() { step = Step.Login; plan = null }

    fun run() {
        val s = server ?: return
        val p = plan ?: return
        step = Step.Running
        error = null
        viewModelScope.launch {
            try {
                val d = withContext(Dispatchers.IO) { s.run(target(), p.disk, listener) }
                if (chatgptJson != null && !land) signOutChatGpt() // the sign-in now belongs to that bia
                done = d
                step = Step.Installed
                saved.clear(); saved.addAll(savedBias())
                val ca = if (land) null else d.caPem
                withContext(Dispatchers.IO) {
                    waitUp(cleanHost(), ca, 25u, object : WaitListener {
                        override fun waited(seconds: ULong) { viewModelScope.launch { waited = seconds.toLong() } }
                    })
                }
                up = true
            } catch (e: Exception) {
                val msg = (e as? DeployException.Failed)?.reason ?: e.message ?: e.toString()
                if (step == Step.Installed) { up = false; upError = msg } else { error = msg; step = Step.Confirm }
            }
        }
    }

    fun again() {
        step = Step.Login; plan = null; facts = null; done = null; up = null; upError = null
        keySaved = false; rebooting = false; progress = ""; fraction = null; log.clear()
        secret = landSecret()
    }

    fun loadKeyText(text: String) { privateKey = text }

    // ---- bia's keys kept on this phone ----
    val saved = mutableStateListOf<SavedBia>().apply { addAll(savedBias()) }

    fun openKeys() {
        saved.clear(); saved.addAll(savedBias())
        page = Page.Keys
    }
}
