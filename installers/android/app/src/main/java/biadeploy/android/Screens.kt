package biadeploy.android

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import android.security.KeyChain
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.bia_deploy_mobile.biaconfigExport
import uniffi.bia_deploy_mobile.biaconfigPassword
import uniffi.bia_deploy_mobile.keyGuides
import uniffi.bia_deploy_mobile.languages
import uniffi.bia_deploy_mobile.p12Password
import java.io.File

/** bia's blue, as in the desktop app: what buttons are filled with, under white text. */
private val Accent = Color(0xFF265CA0)
private val Good = Color(0xFF28A046)

@Composable
fun BiaTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val scheme = if (dark) darkColorScheme(primary = Color(0xFF7FA8E0), secondary = Accent, background = Color(0xFF121318), surface = Color(0xFF1C1D22),
        secondaryContainer = Color(0xFF2A3F5F), onSecondaryContainer = Color(0xFFDCE7F7), surfaceVariant = Color(0xFF2A2C33), onSurfaceVariant = Color(0xFFC6C8D0))
    else lightColorScheme(primary = Accent, secondary = Accent, background = Color(0xFFF2F2F2), surface = Color.White,
        secondaryContainer = Color(0xFFD6E3F5), onSecondaryContainer = Color(0xFF0F2A4D), surfaceVariant = Color(0xFFE8EAEE), onSurfaceVariant = Color(0xFF44474E))
    // the system bars are see-through here: their icons must suit what this app draws under them
    val view = LocalView.current
    if (!view.isInEditMode) SideEffect {
        val window = (view.context as Activity).window
        WindowCompat.getInsetsController(window, view).apply {
            isAppearanceLightStatusBars = !dark
            isAppearanceLightNavigationBars = !dark
        }
    }
    MaterialTheme(colorScheme = scheme, content = content)
}

@Composable
fun App(m: DeployModel) {
    m.configFile?.let { ImportDialog(m) }
    m.chat?.let { target ->
        ChatScreen(target) { m.chat = null }
        return
    }
    BackHandler(enabled = (m.page == Page.Server && m.step == Step.Login) || m.page == Page.Keys) { m.page = Page.Brain }
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        // on a tablet the wizard stays a readable column in the middle; the scrolling stops at the
        // status bar, so the page never runs under its clock and icons
        val scroll = rememberScrollState()
        // demo "name" (store screenshots): the end of the brain page, where name and gender are
        if (m.demoScrollEnd) LaunchedEffect(scroll.maxValue) { scroll.scrollTo(scroll.maxValue) }
        Box(
            Modifier.fillMaxSize()
                .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Top + WindowInsetsSides.Horizontal))
                .verticalScroll(scroll),
            contentAlignment = Alignment.TopCenter,
        ) {
        Column(
            Modifier
                .widthIn(max = 640.dp)
                .fillMaxWidth()
                .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Bottom))
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(verticalAlignment = Alignment.Bottom) {
                Text("bia", fontSize = 34.sp, fontWeight = FontWeight.Bold, color = MaterialTheme.colorScheme.primary)
                Text(" deploy", fontSize = 34.sp, color = MaterialTheme.colorScheme.onBackground.copy(alpha = 0.5f))
                Spacer(Modifier.weight(1f))
                if (m.page == Page.Brain) OutlinedButton({ m.openKeys() }) { Text("Your bias") }
            }
            when (m.page) {
                Page.Brain -> BrainPage(m)
                Page.Server -> ServerPage(m)
                Page.Keys -> KeysPage(m)
            }
            m.error?.let { ErrorCard(it) }
        }
        }
    }
}

@Composable
private fun Card(content: @Composable ColumnScope.() -> Unit) {
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surface, tonalElevation = 1.dp, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp), content = content)
    }
}

@Composable
private fun Title(text: String) = Text(text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)

@Composable
private fun Weak(text: String) = Text(text, color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f), fontSize = 14.sp)

@Composable
private fun Primary(text: String, enabled: Boolean = true, danger: Boolean = false, onClick: () -> Unit) {
    val colors = if (danger) ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error, contentColor = MaterialTheme.colorScheme.onError)
    else ButtonDefaults.buttonColors(containerColor = Accent, contentColor = Color.White)
    Button(onClick, enabled = enabled, shape = RoundedCornerShape(22.dp), colors = colors, modifier = Modifier.fillMaxWidth().height(48.dp)) {
        Text(text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
    }
}

/** Buttons side by side: one kind of button, and those that do not fit go to the next line. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Buttons(content: @Composable () -> Unit) =
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) { content() }

@Composable
private fun ErrorCard(text: String) {
    Surface(shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.errorContainer, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(14.dp)) {
            Text("That did not work", fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onErrorContainer)
            Text(text, color = MaterialTheme.colorScheme.onErrorContainer, fontSize = 14.sp)
        }
    }
}

@Composable
private fun Busy(text: String) = Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
    Text(text)
}

@Composable
private fun Check(done: Boolean, text: String) = Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
    Text(if (done) "✔" else "○", color = if (done) Good else MaterialTheme.colorScheme.primary, fontWeight = FontWeight.Bold)
    Text(text, color = if (done) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f))
}

// ---- brain ----

@Composable
private fun BrainPage(m: DeployModel) {
    val context = LocalContext.current
    Card {
        Title("First, bia's brain")
        Weak("bia thinks with a cloud AI model. Use your ChatGPT plan, or an API key from a model provider.")
        if (m.chatgptEmail != null) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Signed in to ChatGPT as ${m.chatgptEmail}", Modifier.weight(1f))
                TextButton({ m.signOutChatGpt() }) { Text("Sign out") }
            }
        } else {
            val dark = isSystemInDarkTheme()
            Button({ m.signInChatGpt() }, enabled = m.busy == null, shape = RoundedCornerShape(22.dp),
                colors = ButtonDefaults.buttonColors(containerColor = if (dark) Color.White else Color(0xFF101010), contentColor = if (dark) Color(0xFF101010) else Color.White),
                modifier = Modifier.fillMaxWidth().height(48.dp)) {
                Text("Continue with ChatGPT")
            }
            Weak("Or use an API key")
            OutlinedTextField(m.apiKey, { m.keyChanged(it) }, label = { Text("API key") }, singleLine = true,
                visualTransformation = PasswordVisualTransformation(), modifier = Modifier.fillMaxWidth())
            if (m.provider.isNotEmpty()) Weak("Provider: ${m.provider}")
            var help by remember { mutableStateOf(false) }
            TextButton({ help = !help }) { Text(if (help) "Hide" else "How do I get an API key?") }
            if (help) {
                keyGuides().forEach { g ->
                    Column(Modifier.clickable { open(context, g.keyUrl) }.padding(vertical = 4.dp)) {
                        Text("${g.name} — get a key", color = MaterialTheme.colorScheme.primary, fontWeight = FontWeight.SemiBold)
                        Weak(g.about)
                    }
                }
            }
        }
        if (m.provider.isNotEmpty()) {
            OutlinedTextField(m.model, { m.model = it }, label = { Text("Model") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            if (m.models.isNotEmpty()) {
                var query by remember { mutableStateOf("") }
                OutlinedTextField(query, { query = it }, label = { Text("Search models") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                val hits = m.models.filter { query.isBlank() || it.id.contains(query, true) || it.name.contains(query, true) }.take(40)
                Column(Modifier.heightIn(max = 260.dp).verticalScroll(rememberScrollState())) {
                    hits.forEach { md ->
                        val picked = md.id == m.model
                        Text(md.id + if (md.name != md.id) "  — ${md.name}" else "",
                            Modifier.fillMaxWidth()
                                .background(if (picked) Accent.copy(alpha = 0.15f) else Color.Transparent, RoundedCornerShape(8.dp))
                                .clickable { m.model = md.id; if (md.ctx > 0u) m.ctx = md.ctx }
                                .padding(8.dp), fontSize = 14.sp)
                    }
                }
            }
        }
    }
    Card {
        Title("bia's language")
        LanguagePicker(m)
        Weak("bia talks in it from the first boot. Text only: no voice exists, so bia speaks English aloud.")
    }
    Card {
        Title("bia's name")
        OutlinedTextField(m.biaName, { m.biaName = it.take(40) }, label = { Text("Name") }, placeholder = { Text("e.g. Fred") },
            singleLine = true, modifier = Modifier.fillMaxWidth())
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("male" to "Male", "female" to "Female", "neutral" to "Neutral").forEach { (g, label) ->
                FilterChip(m.gender == g, { m.gender = if (m.gender == g) "" else g }, label = { Text(label) })
            }
        }
        Weak("bia always knows its name and gender, and its certificates and web page carry the name. Leave them empty and bia asks on its first start.")
    }
    when {
        m.busy != null -> Busy(m.busy!!)
        m.brainOk -> {
            m.hello?.let { Bubble(it) }
            Primary("Continue  ›") { m.land = false; m.page = Page.Server }
        }
        else -> Primary("Wake bia up", enabled = m.provider.isNotEmpty() && m.model.isNotBlank()) { m.wakeBia() }
    }
    TextButton({ m.land = true; m.page = Page.Server }) {
        Text("No key needed: I am preparing a server that one of my bias will fly its agent to (land)")
    }
}

@Composable
private fun Bubble(text: String) = Surface(shape = RoundedCornerShape(16.dp), color = Accent, modifier = Modifier.fillMaxWidth()) {
    Text("bia:  $text", color = Color.White, fontSize = 17.sp, modifier = Modifier.padding(14.dp))
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LanguagePicker(m: DeployModel) {
    val all = remember { languages() }
    var open by remember { mutableStateOf(false) }
    val current = all.firstOrNull { it.code == m.lang } ?: all.first()
    ExposedDropdownMenuBox(open, { open = it }) {
        OutlinedTextField(current.name + if (current.voice) "" else " (text only)", {}, readOnly = true, label = { Text("Language") },
            modifier = Modifier.fillMaxWidth().menuAnchor(MenuAnchorType.PrimaryNotEditable))
        ExposedDropdownMenu(open, { open = false }) {
            all.forEach { l ->
                DropdownMenuItem({ Text(l.name + if (l.voice) "" else " (text only)") }, { m.lang = l.code; open = false })
            }
        }
    }
}

// ---- server ----

@Composable
private fun ServerPage(m: DeployModel) {
    if (m.step == Step.Login) TextButton({ m.page = Page.Brain }) { Text("‹  Back") }
    when (m.step) {
        Step.Login, Step.Connecting -> LoginStep(m)
        Step.HostKey -> HostKeyStep(m)
        Step.Checking -> Card { Busy("Looking at the server…"); Weak("bia checks the disk, memory and network there. Nothing changes yet.") }
        Step.Confirm -> ConfirmStep(m)
        Step.Running -> RunningStep(m)
        Step.Installed -> InstalledStep(m)
    }
}

@Composable
private fun LoginStep(m: DeployModel) {
    val context = LocalContext.current
    val clip = LocalClipboardManager.current
    val pickKey = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        runCatching { context.contentResolver.openInputStream(uri)!!.use { it.readBytes().decodeToString() } }
            .onSuccess { m.loadKeyText(it) }.onFailure { m.error = it.message }
    }
    Card {
        Title(if (m.land) "Which server will receive an agent?" else "Which server?")
        Weak("A Linux server rented for bia (a VPS). Its provider's page shows its address and how to log in. Everything on it will be replaced by bia.")
        OutlinedTextField(m.host, { m.host = it }, label = { Text("Address") }, placeholder = { Text("203.0.113.10") }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri), modifier = Modifier.fillMaxWidth())
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedTextField(m.user, { m.user = it }, label = { Text("User") }, singleLine = true, modifier = Modifier.weight(2f))
            OutlinedTextField(m.port, { m.port = it }, label = { Text("Port") }, singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.weight(1f))
        }
    }
    Card {
        Title("How will bia log in?")
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            How.entries.forEach { h ->
                FilterChip(m.how == h, { m.how = h }, label = {
                    Text(when (h) { How.Password -> "Password"; How.Key -> "Key file"; How.NewKey -> "New key" })
                })
            }
        }
        when (m.how) {
            How.Password -> {
                OutlinedTextField(m.password, { m.password = it }, label = { Text("The server's password") }, singleLine = true,
                    visualTransformation = PasswordVisualTransformation(), modifier = Modifier.fillMaxWidth())
                Weak("bia uses the password only now, to log in; it is not saved.")
            }
            How.Key -> {
                OutlinedButton({ pickKey.launch(arrayOf("*/*")) }) { Text("Choose the key file…") }
                OutlinedTextField(m.privateKey, { m.privateKey = it }, label = { Text("…or paste the key (BEGIN to END)") },
                    textStyle = LocalTextStyle.current.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp), minLines = 3, maxLines = 6,
                    modifier = Modifier.fillMaxWidth())
                OutlinedTextField(m.passphrase, { m.passphrase = it }, label = { Text("Passphrase (only if the key has one)") }, singleLine = true,
                    visualTransformation = PasswordVisualTransformation(), modifier = Modifier.fillMaxWidth())
            }
            How.NewKey -> {
                Weak("When a server is ordered for bia, the provider asks for an \"SSH key\". bia makes one here: its public half goes to the provider, and bia keeps the secret half on this phone.")
                val k = m.generated
                if (k == null) {
                    Primary("Make a key", enabled = m.busy == null) { m.makeKey() }
                } else {
                    Text("1. Copy this and paste it where the provider asks for an SSH key:", fontWeight = FontWeight.SemiBold)
                    Surface(shape = RoundedCornerShape(8.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                        Text(k.public, fontFamily = FontFamily.Monospace, fontSize = 12.sp, modifier = Modifier.padding(8.dp))
                    }
                    Buttons {
                        OutlinedButton({ clip.setText(AnnotatedString(k.public)) }) { Text("Copy") }
                        OutlinedButton({
                            context.startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, k.public), "SSH key"))
                        }) { Text("Share") }
                        OutlinedButton({ m.makeKey() }) { Text("Make another") }
                    }
                    Text("2. When the server is ready, type its address above and press Connect.", fontWeight = FontWeight.SemiBold)
                }
            }
        }
    }
    if (m.land) LandSecretCard(m)
    if (m.step == Step.Connecting) Busy("connecting…") else Primary("Connect", enabled = m.host.isNotBlank()) { m.connect() }
}

@Composable
private fun LandSecretCard(m: DeployModel) {
    val clip = LocalClipboardManager.current
    Card {
        Title("Land secret")
        Weak("This server becomes an empty bia that waits for one of your bias to fly its agent there. That bia needs this secret:")
        Text(m.secret, fontFamily = FontFamily.Monospace, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
        OutlinedButton({ clip.setText(AnnotatedString(m.secret)) }) { Text("Copy") }
        var own by remember { mutableStateOf(false) }
        TextButton({ own = !own }) { Text("Use the secret my bia made instead") }
        if (own) OutlinedTextField(m.secret, { m.secret = it }, singleLine = true, textStyle = LocalTextStyle.current.copy(fontFamily = FontFamily.Monospace),
            modifier = Modifier.fillMaxWidth())
    }
}

@Composable
private fun HostKeyStep(m: DeployModel) {
    val (known, now) = m.changedKey ?: return
    Card {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Filled.Warning, null, tint = MaterialTheme.colorScheme.error)
            Title("This server's fingerprint changed")
        }
        Text("bia knew it as $known, now it says $now.", fontFamily = FontFamily.Monospace, fontSize = 12.sp)
        Text("A server that was reinstalled gets a new fingerprint. If you did not reinstall it, stop here: someone may be in between.")
        Primary("It was reinstalled: trust the new one", danger = true) { m.connect(accept = now) }
        TextButton({ m.back() }) { Text("Back") }
    }
}

private fun rowName(k: String) = when (k) {
    "mode" -> "bia"; "fmm" -> "brain"; "lang" -> "language"; "self" -> "bia's name"; "root certificate" -> "bia's key"
    "server certificate" -> "names in its certificate"; else -> k
}

@Composable
private fun ConfirmStep(m: DeployModel) {
    val f = m.facts ?: return
    val p = m.plan ?: return
    Card {
        Title("${f.hostname}  ·  ${f.os}")
        Weak("${if (f.arch == "x86_64") "Intel/AMD" else "ARM"} server, ${"%.1f".format(f.ramMib.toDouble() / 1024)} GiB of memory" +
            if (f.rootLogin) "" else "; bia works as root through sudo")
        p.rows.forEach { r ->
            Row { Text(rowName(r.what), Modifier.width(110.dp), color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f), fontSize = 14.sp); Text(r.value, fontSize = 14.sp) }
        }
    }
    var understood by remember { mutableStateOf(false) }
    Card {
        Text("⚠  Everything on ${f.hostname} is erased", color = MaterialTheme.colorScheme.error, fontSize = 19.sp, fontWeight = FontWeight.SemiBold)
        Text("Its disk (${p.disk}) is replaced by bia: files, websites, databases, all of it. " +
            if (p.land) "Afterwards bia waits for one of your bias to fly its agent there with the land secret."
            else "Afterwards you reach bia in the browser, with the key this app keeps for you.")
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.clickable { understood = !understood }) {
            Checkbox(understood, { understood = it })
            Text("I understand, nothing on this server needs to be kept")
        }
        Primary("Replace ${f.hostname} with bia", enabled = understood, danger = true) { m.run() }
        TextButton({ m.back() }) { Text("Back") }
    }
    if (p.land) LandSecretCard(m)
}

@Composable
private fun RunningStep(m: DeployModel) = Card {
    Title("Putting bia on the server")
    Busy(m.progress.ifEmpty { "starting…" })
    m.fraction?.let { LinearProgressIndicator(progress = { it }, modifier = Modifier.fillMaxWidth()) }
    if (!m.land) Check(m.keySaved, "bia's key (root's certificate) is saved on this phone")
    Check(m.rebooting, "the server restarts into bia's installer")
    Weak("Keep this app open. Nothing on the server changes before bia's key is safe here.")
}

@Composable
private fun InstalledStep(m: DeployModel) {
    val context = LocalContext.current
    val d = m.done ?: return
    val host = m.cleanHost()
    val url = "https://${if (host.contains(':')) "[$host]" else host}/"
    Card {
        when (m.up) {
            null -> {
                Title("bia is installing itself")
                Busy("The server writes bia onto its disk and starts it. This takes a few minutes… ${m.waited / 60}:${"%02d".format(m.waited % 60)}")
            }
            true -> {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Icon(Icons.Filled.CheckCircle, null, tint = Good)
                    Text("bia is up!", fontSize = 22.sp, fontWeight = FontWeight.SemiBold, color = Good)
                }
                if (m.land) Text("It waits at $host for your bia. Tell your bia: fly to $host with land secret ${m.secret}")
                else {
                    d.folder?.let { folder -> Primary("Open bia's chat") { m.chat = ChatTarget(host, folder) } }
                    TextButton({ open(context, url) }) { Text("Open in a browser instead") }
                }
            }
            false -> {
                Title("bia did not answer yet")
                Text(m.upError ?: "")
                if (!m.land) OutlinedButton({ open(context, url) }) { Text("Try to open it anyway") }
            }
        }
    }
    d.folder?.let { folder -> KeyCard(m, folder, host) }
    if (m.land) LandSecretCard(m)
    TextButton({ m.again() }) { Text("Set up another server") }
}

@Composable
private fun KeyCard(m: DeployModel, folder: String, host: String, savedAt: ULong? = null, name: String = "") {
    val context = LocalContext.current
    var exporting by remember { mutableStateOf(false) }
    if (exporting) ExportDialog(folder, host, name) { exporting = false }
    var removing by remember { mutableStateOf(false) }
    if (removing) AlertDialog(
        onDismissRequest = { removing = false },
        title = { Text("Remove ${name.ifEmpty { "bia at $host" }} from this phone?") },
        text = { Text("This deletes the key kept on this phone. bia keeps running on its server. If this is the only copy of root's key, root is locked out of that bia for good.") },
        confirmButton = { TextButton({ removing = false; m.forget(folder) }) { Text("Remove", color = MaterialTheme.colorScheme.error) } },
        dismissButton = { TextButton({ removing = false }) { Text("Cancel") } },
    )
    var caSaved by remember { mutableStateOf(false) }
    val saveCa = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/x-x509-ca-cert")) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        runCatching { context.contentResolver.openOutputStream(uri)!!.use { it.write(File(folder, "ca.crt").readBytes()) } }.onSuccess { caSaved = true }
    }
    val saveP12 = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/x-pkcs12")) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        runCatching { context.contentResolver.openOutputStream(uri)!!.use { it.write(File(folder, "bia-root.p12").readBytes()) } }
    }
    // in the list of saved keys: one short card per bia (the page explains them once)
    if (savedAt != null) {
        Card {
            Title(name.ifEmpty { "bia at $host" })
            if (name.isNotEmpty()) Weak(host)
            Weak("key saved " + java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date(savedAt.toLong() * 1000)))
            Primary("Open chat") { m.chat = ChatTarget(host, folder) }
            Weak("For a browser:")
            Buttons {
                OutlinedButton({ installKey(context, folder, host) }) { Text("Install") }
                OutlinedButton({ share(context, folder, host) }) { Text("Share…") }
                OutlinedButton({ saveP12.launch("bia-$host.p12") }) { Text("Save a copy…") }
            }
            OutlinedButton({ exporting = true }) { Text("Share as .biaconfig…") }
            TextButton({ removing = true }) { Text("Remove from this phone…", color = MaterialTheme.colorScheme.error) }
        }
        return
    }
    Card {
        Title("bia's key")
        Text("bia-$host.p12 is root's certificate: the only way into this bia. Keep a copy somewhere safe (password ${p12Password()}).")
        Weak("bia's chat in this app uses it by itself. The steps below are for a browser on this phone.")
        Text("1. Install it, so the browser can show it to bia:", fontWeight = FontWeight.SemiBold)
        OutlinedButton({ installKey(context, folder, host) }) { Text("Install bia's key") }
        Text("2. Trust bia's certificate authority, so the browser opens bia without a warning:", fontWeight = FontWeight.SemiBold)
        Weak("Android lets apps only hand it over: save it, then in Settings › Security › Encryption & credentials › Install a certificate › CA certificate, pick the saved file.")
        Buttons {
            OutlinedButton({ saveCa.launch("bia-$host-ca.crt") }) { Text(if (caSaved) "Saved ✔" else "Save it") }
            OutlinedButton({
                try { context.startActivity(Intent(Settings.ACTION_SECURITY_SETTINGS)) } catch (_: ActivityNotFoundException) {}
            }) { Text("Open Settings") }
        }
        Buttons {
            OutlinedButton({ share(context, folder, host) }) { Text("Share bia's key…") }
            OutlinedButton({ saveP12.launch("bia-$host.p12") }) { Text("Save a copy…") }
        }
        Weak("Share sends bia-$host.p12 to a computer or another phone (mail, Nearby Share, a chat to yourself); it asks for the password above when it is installed.")
        OutlinedButton({ exporting = true }) { Text("Share as .biaconfig…") }
        Weak("A .biaconfig also carries bia's address and is locked with its own password: another bia deploy app opens it and gets the chat.")
    }
}

/** bia's key into Android's credential store, for the browser to show to bia. */
private fun installKey(context: android.content.Context, folder: String, host: String) {
    val i = KeyChain.createInstallIntent()
        .putExtra(KeyChain.EXTRA_PKCS12, File(folder, "bia-root.p12").readBytes())
        .putExtra(KeyChain.EXTRA_NAME, "bia $host")
    runCatching { context.startActivity(i) }
}

/** bia's key (and its CA) to the share sheet: copies named after the host, from the cache. */
private fun share(context: android.content.Context, folder: String, host: String) {
    runCatching {
        val dir = File(context.cacheDir, "share").apply { deleteRecursively(); mkdirs() }
        val auth = "${context.packageName}.share"
        val files = listOf("bia-root.p12" to "bia-$host.p12", "ca.crt" to "bia-$host-ca.crt")
            .filter { File(folder, it.first).exists() }
            .map { (from, to) -> File(folder, from).copyTo(File(dir, to), overwrite = true) }
        val uris = ArrayList(files.map { androidx.core.content.FileProvider.getUriForFile(context, auth, it) })
        val send = Intent(Intent.ACTION_SEND_MULTIPLE)
            .setType("application/octet-stream")
            .putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
            .putExtra(Intent.EXTRA_SUBJECT, "bia $host: root's key")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        send.clipData = android.content.ClipData.newUri(context.contentResolver, "bia key", uris[0]).apply { uris.drop(1).forEach { addItem(android.content.ClipData.Item(it)) } }
        context.startActivity(Intent.createChooser(send, "Share bia's key"))
    }
}

/** Every bia this phone deployed: its key, to install or share again later. */
@Composable
private fun KeysPage(m: DeployModel) {
    val context = LocalContext.current
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        m.offerConfig(context.cacheDir) { dest -> context.contentResolver.openInputStream(uri)?.use { it.copyLimited(dest) } }
    }
    TextButton({ m.page = Page.Brain }) { Text("‹  Back") }
    Card {
        Title("Your bias")
        OutlinedButton({ pick.launch(arrayOf("*/*")) }) { Text("Import a .biaconfig…") }
        if (m.saved.isEmpty()) Text("No bia's keys on this phone yet: they are kept here after a deploy or an import.")
        else Weak("Each is a certificate for one bia, the way into it (password ${p12Password()}). Install puts it on this phone for the browser; Share sends bia-<address>.p12 and its CA to a computer or another phone.")
    }
    m.saved.forEach { b -> KeyCard(m, b.folder, b.host, b.savedAt, b.name) }
}

/** A .biaconfig opened on this phone: its password, then what happens to a key already here. */
@Composable
private fun ImportDialog(m: DeployModel) {
    var password by remember { mutableStateOf("") }
    val other = m.configOtherKey
    if (other != null) {
        AlertDialog(
            onDismissRequest = { m.closeConfig() },
            title = { Text("Replace the key for $other?") },
            text = { Text("This phone already has a different key for $other. If it is root's and no copy exists elsewhere, replacing it can lock root out of that bia.") },
            confirmButton = { TextButton({ m.configOtherKey = null; m.importConfig(password, true) }) { Text("Replace", color = MaterialTheme.colorScheme.error) } },
            dismissButton = { TextButton({ m.closeConfig() }) { Text("Keep the old key") } },
        )
        return
    }
    AlertDialog(
        onDismissRequest = { if (!m.configBusy) m.closeConfig() },
        title = { Text("Open .biaconfig") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("It holds a key to a bia and that bia's address. Only import a file from someone trusted.")
                OutlinedTextField(
                    password, { password = it },
                    label = { Text("Password") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                    enabled = !m.configBusy,
                )
                m.configProblem?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = { TextButton({ m.importConfig(password) }, enabled = !m.configBusy && password.isNotEmpty()) { Text(if (m.configBusy) "Opening…" else "Import") } },
        dismissButton = { TextButton({ m.closeConfig() }, enabled = !m.configBusy) { Text("Cancel") } },
    )
}

/** This phone's key as a .biaconfig, for the share sheet. */
@Composable
private fun ExportDialog(folder: String, host: String, name: String, close: () -> Unit) {
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    val scope = rememberCoroutineScope()
    var password by remember { mutableStateOf(runCatching { biaconfigPassword() }.getOrDefault("")) }
    var problem by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = { if (!busy) close() },
        title = { Text("Share as .biaconfig") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("Whoever opens it holds this same certificate, with everything it can do on this bia. Revoking it later ends it for this phone too.")
                OutlinedTextField(password, { password = it }, label = { Text("Password") }, singleLine = true, enabled = !busy)
                TextButton({ clipboard.setText(AnnotatedString(password)) }) { Text("Copy the password") }
                Weak("Send the password another way, never next to the file.")
                problem?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton({
                busy = true
                problem = null
                scope.launch {
                    try {
                        val dir = File(context.cacheDir, "share").apply { deleteRecursively(); mkdirs() }
                        val path = withContext(Dispatchers.IO) { biaconfigExport(folder, host, name, password, dir.path) }
                        val uri = androidx.core.content.FileProvider.getUriForFile(context, "${context.packageName}.share", File(path))
                        val send = Intent(Intent.ACTION_SEND)
                            .setType("application/x-biaconfig")
                            .putExtra(Intent.EXTRA_STREAM, uri)
                            .putExtra(Intent.EXTRA_SUBJECT, "bia $host")
                            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        send.clipData = android.content.ClipData.newUri(context.contentResolver, "bia config", uri)
                        context.startActivity(Intent.createChooser(send, "Share .biaconfig"))
                        close()
                    } catch (e: uniffi.bia_deploy_mobile.DeployException.Failed) {
                        problem = e.reason
                    } catch (e: Exception) {
                        problem = e.message ?: e.toString()
                    } finally {
                        busy = false
                    }
                }
            }, enabled = !busy && password.isNotEmpty()) { Text(if (busy) "Locking…" else "Share…") }
        },
        dismissButton = { TextButton(close, enabled = !busy) { Text("Cancel") } },
    )
}

private fun open(context: android.content.Context, url: String) {
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}
