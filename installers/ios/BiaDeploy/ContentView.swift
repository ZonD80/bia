import SwiftUI
import UniformTypeIdentifiers

/// bia's blue for what is filled, under white text.
private let accentFill = Color(red: 0.149, green: 0.361, blue: 0.627)
/// bia's blue for words and outlines: the filled blue is too dark to read on a dark background.
let accent = Color(UIColor { $0.userInterfaceStyle == .dark
    ? UIColor(red: 0.498, green: 0.659, blue: 0.878, alpha: 1)
    : UIColor(red: 0.149, green: 0.361, blue: 0.627, alpha: 1) })
private let good = Color(red: 0.157, green: 0.627, blue: 0.275)

struct ContentView: View {
    @ObservedObject var model: DeployModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Text("bia").font(.system(size: 34, weight: .bold)).foregroundStyle(accent)
                    Spacer()
                    if model.page == .keys && !model.saved.isEmpty { Button("Start new bia") { model.page = .brain }.buttonStyle(.bordered).tint(accent) }
                }
                switch model.page {
                case .brain: BrainPage(model: model)
                case .server: ServerPage(model: model)
                case .keys: KeysPage(model: model)
                }
                if let e = model.error { ErrorCard(text: e) }
            }
            .padding(16)
            // on an iPad the wizard stays a readable column in the middle
            .frame(maxWidth: 640)
            .frame(maxWidth: .infinity)
        }
        // the page stops at the status bar, so it never runs under the clock and icons
        .safeAreaInset(edge: .top, spacing: 0) {
            Color.clear.frame(height: 0).background(Color(.systemGroupedBackground))
        }
        .background(Color(.systemGroupedBackground))
        .scrollDismissesKeyboard(.interactively)
        .overlay {
            if model.gettingKey {
                ProgressView("Getting bia's key…").padding(24).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
            }
        }
        .fullScreenCover(item: $model.chat) { ChatPage(target: $0) }
        .sheet(item: $model.configFile, onDismiss: { model.closeConfig() }) { _ in ImportSheet(model: model) }
        .sheet(item: $model.noKey) { NoKeySheet(model: model, link: $0) }
        #if DEBUG
        .task {
            // for testing: `-importConfig <path> -importPassword <password> [-importReplace YES]`
            if let path = UserDefaults.standard.string(forKey: "importConfig") {
                model.offerConfig(URL(fileURLWithPath: path))
                if let pw = UserDefaults.standard.string(forKey: "importPassword") {
                    model.importConfig(password: pw, replace: UserDefaults.standard.bool(forKey: "importReplace"))
                }
            }
            // for testing: `-exportConfig <host> -exportPassword <password> -exportTo <dir>`
            if let host = UserDefaults.standard.string(forKey: "exportConfig"), let b = savedBias().first(where: { $0.host == host }),
               let pw = UserDefaults.standard.string(forKey: "exportPassword"), let to = UserDefaults.standard.string(forKey: "exportTo") {
                NSLog("bia export: %@", (try? biaconfigExport(folder: b.folder, host: b.host, name: b.name, password: pw, outDir: to)) ?? "failed")
            }
            // a link from bia, for testing: `-link <url>`
            if let l = UserDefaults.standard.string(forKey: "link"), let u = URL(string: l) { model.openLink(u) }
            // store screenshots: `-demo <scene>` (made-up data, no network)
            // a saved bia's chat at once, for testing: `-chat <host>`
            if let host = UserDefaults.standard.string(forKey: "chat"), let b = model.saved.first(where: { $0.host == host }) {
                model.chat = ChatTarget(host: b.host, folder: b.folder)
            }
            if let scene = UserDefaults.standard.string(forKey: "demo") {
                let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                    .appendingPathComponent("bia-deploy", isDirectory: true)
                model.demo(scene, appDir: dir)
            }
        }
        #endif
    }
}

struct Card<Content: View>: View {
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 10) { content }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16))
    }
}

struct PrimaryButton: View {
    let title: String
    var color: Color = accentFill
    var enabled = true
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Text(title).font(.headline).frame(maxWidth: .infinity).frame(height: 48)
        }
        .buttonStyle(.borderedProminent)
        .tint(color)
        .clipShape(RoundedRectangle(cornerRadius: 22))
        .disabled(!enabled)
    }
}

struct ErrorCard: View {
    let text: String
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("That did not work").font(.headline)
            Text(text).font(.subheadline)
        }
        .foregroundStyle(.red)
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.red.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
    }
}

struct Busy: View {
    let text: String
    var body: some View { HStack(spacing: 10) { ProgressView(); Text(text) } }
}

struct Check: View {
    let done: Bool
    let text: String
    var body: some View {
        HStack(spacing: 8) {
            Text(done ? "✔" : "○").bold().foregroundStyle(done ? good : accent)
            Text(text).foregroundStyle(done ? .primary : .secondary)
        }
    }
}

// MARK: - brain

struct BrainPage: View {
    @ObservedObject var model: DeployModel
    @State private var help = false
    @State private var query = ""

    var body: some View {
        Button("‹  Back") { model.openKeys() }
        Card {
            Text("First, bia's brain").font(.title2.weight(.semibold))
            Text("bia thinks with a cloud AI model. Use your ChatGPT plan, or an API key from a model provider.").foregroundStyle(.secondary)
            if let email = model.chatgptEmail {
                HStack {
                    Text("Signed in to ChatGPT as \(email)")
                    Spacer()
                    Button("Sign out") { model.signOutChatGpt() }
                }
            } else {
                Button { model.signInChatGpt() } label: {
                    Text("Continue with ChatGPT").font(.headline).frame(maxWidth: .infinity).frame(height: 48)
                        .foregroundStyle(Color(.systemBackground))
                }
                .buttonStyle(.borderedProminent).tint(Color(.label)).clipShape(RoundedRectangle(cornerRadius: 22))
                .disabled(model.busy != nil)
                Text("Or use an API key").foregroundStyle(.secondary)
                SecureField("API key", text: $model.apiKey)
                    .textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                    .onChange(of: model.apiKey) { _ in model.keyChanged() }
                if !model.provider.isEmpty { Text("Provider: \(model.provider)").font(.subheadline).foregroundStyle(.secondary) }
                Button(help ? "Hide" : "How do I get an API key?") { help.toggle() }
                if help {
                    ForEach(keyGuides(), id: \.provider) { g in
                        VStack(alignment: .leading, spacing: 2) {
                            if let u = URL(string: g.keyUrl) { Link("\(g.name) — get a key", destination: u).bold() }
                            Text(g.about).font(.footnote).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            if !model.provider.isEmpty {
                TextField("Model", text: $model.model).textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                if !model.models.isEmpty {
                    TextField("Search models", text: $query).textFieldStyle(.roundedBorder)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    let hits = model.models.filter { query.isEmpty || $0.id.localizedCaseInsensitiveContains(query) || $0.name.localizedCaseInsensitiveContains(query) }.prefix(40)
                    ScrollView {
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(Array(hits), id: \.id) { m in
                                Button { model.model = m.id; if m.ctx > 0 { model.ctx = m.ctx } } label: {
                                    Text(m.id + (m.name != m.id ? "  — \(m.name)" : "")).font(.subheadline)
                                        .frame(maxWidth: .infinity, alignment: .leading).padding(8)
                                        .background(m.id == model.model ? accent.opacity(0.15) : .clear, in: RoundedRectangle(cornerRadius: 8))
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                    .frame(maxHeight: 240)
                }
            }
        }
        Card {
            Text("bia's language").font(.title3.weight(.semibold))
            Picker("Language", selection: $model.lang) {
                ForEach(languages(), id: \.code) { l in Text(l.name + (l.voice ? "" : " (text only)")).tag(l.code) }
            }
            Text("bia talks in it from the first boot. Text only: no voice exists, so bia speaks English aloud.").font(.footnote).foregroundStyle(.secondary)
        }
        Card {
            Text("bia's name").font(.title3.weight(.semibold))
            TextField("Name, e.g. Fred", text: $model.biaName)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
            Picker("Gender", selection: $model.gender) {
                Text("Ask later").tag("")
                Text("Male").tag("male")
                Text("Female").tag("female")
                Text("Neutral").tag("neutral")
            }
            .pickerStyle(.segmented)
            Text("bia always knows its name and gender, and its certificates and web page carry the name. Leave them empty and bia asks on its first start.").font(.footnote).foregroundStyle(.secondary)
        }
        if let b = model.busy {
            Busy(text: b)
        } else if model.brainOk {
            if let h = model.hello {
                Text("bia:  \(h)").font(.title3).foregroundStyle(.white).padding(14)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(accentFill, in: RoundedRectangle(cornerRadius: 16))
            }
            PrimaryButton(title: "Continue  ›") { model.land = false; model.page = .server }
        } else {
            PrimaryButton(title: "Wake bia up", enabled: !model.provider.isEmpty && !model.model.isEmpty) { model.wakeBia() }
        }
        Button("No key needed: I am preparing a server that one of my bias will fly its agent to (land)") {
            model.land = true; model.page = .server
        }
        .font(.subheadline)
    }
}

// MARK: - server

struct ServerPage: View {
    @ObservedObject var model: DeployModel
    var body: some View {
        if model.step == .login { Button("‹  Back") { model.page = .brain } }
        switch model.step {
        case .login, .connecting: LoginStep(model: model)
        case .hostKey: HostKeyStep(model: model)
        case .checking:
            Card {
                Busy(text: "Looking at the server…")
                Text("bia checks the disk, memory and network there. Nothing changes yet.").foregroundStyle(.secondary)
            }
        case .confirm: ConfirmStep(model: model)
        case .running: RunningStep(model: model)
        case .installed: InstalledStep(model: model)
        }
    }
}

struct LoginStep: View {
    @ObservedObject var model: DeployModel
    @State private var picking = false

    var body: some View {
        Card {
            Text(model.land ? "Which server will receive an agent?" : "Which server?").font(.title2.weight(.semibold))
            Text("A Linux server rented for bia (a VPS). Its provider's page shows its address and how to log in. Everything on it will be replaced by bia.").foregroundStyle(.secondary)
            TextField("Address, like 203.0.113.10", text: $model.host).textFieldStyle(.roundedBorder)
                .keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
            HStack {
                TextField("User", text: $model.user).textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                TextField("Port", text: $model.port).textFieldStyle(.roundedBorder).keyboardType(.numberPad).frame(width: 80)
            }
        }
        Card {
            Text("How will bia log in?").font(.title3.weight(.semibold))
            Picker("Login", selection: $model.how) {
                ForEach(How.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            switch model.how {
            case .password:
                SecureField("The server's password", text: $model.password).textFieldStyle(.roundedBorder)
                Text("bia uses the password only now, to log in; it is not saved.").font(.footnote).foregroundStyle(.secondary)
            case .key:
                Button("Choose the key file…") { picking = true }.buttonStyle(.bordered)
                    .fileImporter(isPresented: $picking, allowedContentTypes: [.data, .text, .item]) { r in
                        if case .success(let url) = r {
                            let ok = url.startAccessingSecurityScopedResource()
                            defer { if ok { url.stopAccessingSecurityScopedResource() } }
                            if let t = try? String(contentsOf: url) { model.privateKey = t }
                        }
                    }
                Text("…or paste the key (BEGIN to END):").font(.footnote)
                TextEditor(text: $model.privateKey).font(.system(size: 12, design: .monospaced)).frame(height: 90)
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color(.separator)))
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                SecureField("Passphrase (only if the key has one)", text: $model.passphrase).textFieldStyle(.roundedBorder)
            case .newKey:
                Text("When a server is ordered for bia, the provider asks for an \"SSH key\". bia makes one here: its public half goes to the provider, and bia keeps the secret half on this phone.").font(.subheadline)
                if let k = model.generated {
                    Text("1. Copy this and paste it where the provider asks for an SSH key:").bold()
                    Text(k.public).font(.system(size: 12, design: .monospaced)).textSelection(.enabled)
                        .padding(8).background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8))
                    HStack {
                        Button("Copy") { UIPasteboard.general.string = k.public }.buttonStyle(.bordered)
                        ShareLink(item: k.public) { Text("Share") }.buttonStyle(.bordered)
                        Button("Make another") { model.makeKey() }.buttonStyle(.bordered)
                    }
                    Text("2. When the server is ready, type its address above and press Connect.").bold()
                } else {
                    PrimaryButton(title: "Make a key", enabled: model.busy == nil) { model.makeKey() }
                }
            }
        }
        if model.land { LandSecretCard(model: model) }
        if model.step == .connecting {
            Busy(text: "connecting…")
        } else {
            PrimaryButton(title: "Connect", enabled: !model.host.trimmingCharacters(in: .whitespaces).isEmpty) { model.connect() }
        }
    }
}

struct LandSecretCard: View {
    @ObservedObject var model: DeployModel
    @State private var own = false
    var body: some View {
        Card {
            Text("Land secret").font(.title3.weight(.semibold))
            Text("This server becomes an empty bia that waits for one of your bias to fly its agent there. That bia needs this secret:")
            Text(model.secret).font(.system(size: 17, weight: .semibold, design: .monospaced)).textSelection(.enabled)
            Button("Copy") { UIPasteboard.general.string = model.secret }.buttonStyle(.bordered)
            Button("Use the secret my bia made instead") { own.toggle() }.font(.subheadline)
            if own {
                TextField("Land secret", text: $model.secret).textFieldStyle(.roundedBorder).font(.system(.body, design: .monospaced))
                    .textInputAutocapitalization(.characters).autocorrectionDisabled()
            }
        }
    }
}

struct HostKeyStep: View {
    @ObservedObject var model: DeployModel
    var body: some View {
        if let k = model.changedKey {
            Card {
                Label("This server's fingerprint changed", systemImage: "exclamationmark.triangle.fill").font(.title3.weight(.semibold)).foregroundStyle(.red)
                Text("bia knew it as \(k.known), now it says \(k.now).").font(.system(size: 12, design: .monospaced))
                Text("A server that was reinstalled gets a new fingerprint. If you did not reinstall it, stop here: someone may be in between.")
                PrimaryButton(title: "It was reinstalled: trust the new one", color: .red) { model.connect(accept: k.now) }
                Button("Back") { model.back() }
            }
        }
    }
}

private func rowName(_ k: String) -> String {
    switch k {
    case "mode": return "bia"
    case "fmm": return "brain"
    case "lang": return "language"
    case "self": return "bia's name"
    case "root certificate": return "bia's key"
    case "server certificate": return "names in its certificate"
    default: return k
    }
}

struct ConfirmStep: View {
    @ObservedObject var model: DeployModel
    @State private var understood = false
    var body: some View {
        if let f = model.facts, let p = model.plan {
            Card {
                Text("\(f.hostname)  ·  \(f.os)").font(.title3.weight(.semibold))
                Text("\(f.arch == "x86_64" ? "Intel/AMD" : "ARM") server, \(String(format: "%.1f", Double(f.ramMib) / 1024)) GiB of memory\(f.rootLogin ? "" : "; bia works as root through sudo")")
                    .foregroundStyle(.secondary)
                ForEach(Array(p.rows.enumerated()), id: \.offset) { _, r in
                    HStack(alignment: .top) {
                        Text(rowName(r.what)).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
                        Text(r.value)
                    }
                    .font(.subheadline)
                }
            }
            Card {
                Text("⚠  Everything on \(f.hostname) is erased").font(.title3.weight(.semibold)).foregroundStyle(.red)
                Text("Its disk (\(p.disk)) is replaced by bia: files, websites, databases, all of it. " +
                     (p.land ? "Afterwards bia waits for one of your bias to fly its agent there with the land secret."
                             : "Afterwards you reach bia in the browser, with the key this app keeps for you."))
                Toggle("I understand, nothing on this server needs to be kept", isOn: $understood)
                PrimaryButton(title: "Replace \(f.hostname) with bia", color: .red, enabled: understood) { model.run() }
                Button("Back") { model.back() }
            }
            if p.land { LandSecretCard(model: model) }
        }
    }
}

struct RunningStep: View {
    @ObservedObject var model: DeployModel
    var body: some View {
        Card {
            Text("Putting bia on the server").font(.title3.weight(.semibold))
            Busy(text: model.progress.isEmpty ? "starting…" : model.progress)
            if let f = model.fraction { ProgressView(value: f) }
            if !model.land { Check(done: model.keySaved, text: "bia's key (root's certificate) is saved on this phone") }
            Check(done: model.rebooting, text: "the server restarts into bia's installer")
            Text("Keep this app open. Nothing on the server changes before bia's key is safe here.").font(.footnote).foregroundStyle(.secondary)
        }
    }
}

struct InstalledStep: View {
    @ObservedObject var model: DeployModel
    @Environment(\.openURL) private var openURL

    var body: some View {
        let host = model.cleanHost
        let url = URL(string: "https://\(host.contains(":") ? "[\(host)]" : host)/")
        Card {
            switch model.up {
            case nil:
                Text("bia is installing itself").font(.title3.weight(.semibold))
                Busy(text: "The server writes bia onto its disk and starts it. This takes a few minutes… \(model.waited / 60):\(String(format: "%02d", model.waited % 60))")
            case true?:
                Label("bia is up!", systemImage: "checkmark.circle.fill").font(.title2.weight(.semibold)).foregroundStyle(good)
                if model.land {
                    Text("It waits at \(host) for your bia. Tell your bia: fly to \(host) with land secret \(model.secret)")
                } else if let u = url {
                    if let folder = model.done?.folder {
                        PrimaryButton(title: "Open bia's chat") { model.chat = ChatTarget(host: host, folder: folder) }
                    }
                    Button("Open in a browser instead") { openURL(u) }
                }
            case false?:
                Text("bia did not answer yet").font(.title3.weight(.semibold))
                Text(model.upError ?? "")
                if !model.land, let u = url { Button("Try to open it anyway") { openURL(u) }.buttonStyle(.bordered) }
            }
        }
        if let folder = model.done?.folder { KeyCard(model: model, folder: folder, host: host) }
        if model.land { LandSecretCard(model: model) }
        Button("Set up another server") { model.again() }
        Button("Your bias") { model.openKeys() }
    }
}

/// bia's key for one bia: install it on this phone, or share it (to a computer, another phone).
struct KeyCard: View {
    @ObservedObject var model: DeployModel
    let folder: String
    let host: String
    /// set in the list of saved keys: one short card per bia (the page explains them once)
    var savedAt: Date? = nil
    var name: String = ""
    @State private var exporting = false
    @State private var removing = false

    var body: some View {
        content.sheet(isPresented: $exporting) { ExportSheet(folder: folder, host: host, name: name) }
            .alert("Remove \(name.isEmpty ? "bia at \(host)" : name) from this phone?", isPresented: $removing) {
                Button("Remove", role: .destructive) { model.forget(folder: folder) }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This deletes the key kept on this phone. bia keeps running on its server. If this is the only copy of root's key, root is locked out of that bia for good.")
            }
    }

    @ViewBuilder private var content: some View {
        if let savedAt {
            Card {
                Text(name.isEmpty ? "bia at \(host)" : name).font(.title3.weight(.semibold))
                if !name.isEmpty { Text(host).font(.subheadline).foregroundStyle(.secondary) }
                Text("key saved \(savedAt.formatted(date: .abbreviated, time: .omitted))").font(.subheadline).foregroundStyle(.secondary)
                PrimaryButton(title: "Open chat") { model.chat = ChatTarget(host: host, folder: folder) }
                Text("For a browser:").font(.footnote).foregroundStyle(.secondary)
                HStack {
                    Button("Install") { model.installProfile(folder: folder, host: host) }.buttonStyle(.bordered)
                    ShareLink(items: shareCopies(folder: folder, host: host), subject: Text("bia \(host): root's key")) {
                        Label("Share…", systemImage: "square.and.arrow.up")
                    }
                    .buttonStyle(.bordered)
                }
                Button("Share as .biaconfig…") { exporting = true }.buttonStyle(.bordered)
                Button("Remove from this phone…", role: .destructive) { removing = true }
            }
        } else {
            full
        }
    }

    private var full: some View {
        Card {
            Text("bia's key").font(.title3.weight(.semibold))
            Text("bia-\(host).p12 is root's certificate: the only way into this bia. Keep a copy somewhere safe (password \(p12Password())).")
            Text("bia's chat in this app uses it by itself. The steps below are for a browser.").font(.subheadline).foregroundStyle(.secondary)
            Text("Install it on this phone: Safari offers it as a profile. Then Settings › General › VPN & Device Management installs it, and Settings › General › About › Certificate Trust Settings turns on full trust for \"bia \(host) certificate authority\".")
                .font(.subheadline)
            PrimaryButton(title: "Install bia's key") { model.installProfile(folder: folder, host: host) }
            ShareLink(items: shareCopies(folder: folder, host: host), subject: Text("bia \(host): root's key")) {
                Label("Share bia's key…", systemImage: "square.and.arrow.up")
            }
            .buttonStyle(.bordered)
            Text("Share sends it to a computer or another phone (AirDrop, Mail, Files); it asks for the password above when it is installed.")
                .font(.footnote).foregroundStyle(.secondary)
            Button("Share as .biaconfig…") { exporting = true }.buttonStyle(.bordered)
            Text("A .biaconfig also carries bia's address and is locked with its own password: another bia app opens it and gets the chat.")
                .font(.footnote).foregroundStyle(.secondary)
        }
    }
}

/// Copies named after the host (bia-<host>.p12, bia-<host>-ca.crt) for the share sheet.
func shareCopies(folder: String, host: String) -> [URL] {
    let fm = FileManager.default
    let dir = fm.temporaryDirectory.appendingPathComponent("share-\(host)", isDirectory: true)
    try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
    return [("bia-root.p12", "bia-\(host).p12"), ("ca.crt", "bia-\(host)-ca.crt")].compactMap { from, to in
        let src = URL(fileURLWithPath: folder).appendingPathComponent(from)
        let dst = dir.appendingPathComponent(to)
        guard fm.fileExists(atPath: src.path) else { return nil }
        try? fm.removeItem(at: dst)
        return (try? fm.copyItem(at: src, to: dst)) != nil ? dst : nil
    }
}

/// The first page: every bia whose key this phone keeps (deployed here or imported), with its
/// chat, and its key to install or share again later.
struct KeysPage: View {
    @ObservedObject var model: DeployModel
    @State private var picking = false
    var body: some View {
        Card {
            Text("Your bias").font(.title3.weight(.semibold))
            Button("Import a .biaconfig…") { picking = true }.buttonStyle(.bordered)
                .fileImporter(isPresented: $picking, allowedContentTypes: [UTType(exportedAs: "biadeploy.config")]) { result in
                    if case .success(let url) = result { model.offerConfig(url) }
                }
            if model.saved.isEmpty {
                Text("No bia's keys on this phone yet: they are kept here after a deploy or an import.")
                PrimaryButton(title: "Start new bia") { model.page = .brain }
            } else {
                Text("Each is a certificate for one bia, the way into it (password \(p12Password())). Install offers it to this device as a profile; Share sends bia-<address>.p12 and its CA to a computer or another phone.")
                    .font(.subheadline).foregroundStyle(.secondary)
            }
        }
        ForEach(model.saved, id: \.folder) { b in
            KeyCard(model: model, folder: b.folder, host: b.host, savedAt: Date(timeIntervalSince1970: TimeInterval(b.savedAt)), name: b.name)
        }
    }
}

/// A .biaconfig opened on this phone: its password, then what happens to a key already here.
struct ImportSheet: View {
    @ObservedObject var model: DeployModel
    @State private var password = ""
    var body: some View {
        NavigationStack {
            Form {
                if let other = model.configOtherKey {
                    Section("Replace the key for \(other)?") {
                        Text("This phone already has a different key for \(other). If it is root's and no copy exists elsewhere, replacing it can lock root out of that bia.")
                        Button("Replace", role: .destructive) {
                            model.configOtherKey = nil
                            model.importConfig(password: password, replace: true)
                        }
                        Button("Keep the old key") { model.closeConfig() }
                    }
                } else {
                    Section {
                        Text("It holds a key to a bia and that bia's address. Only import a file from someone trusted.")
                        SecureField("Password", text: $password)
                        if let p = model.configProblem { Text(p).foregroundStyle(.red) }
                    }
                    Section {
                        Button(model.configBusy ? "Opening…" : "Import") { model.importConfig(password: password) }
                            .disabled(model.configBusy || password.isEmpty)
                    }
                }
            }
            .navigationTitle("Open .biaconfig")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { model.closeConfig() }.disabled(model.configBusy) } }
        }
    }
}

extension AppLink: Identifiable { public var id: String { ca + path } }

/// A link from a bia whose key this phone does not hold: bia makes a .biaconfig for it.
struct NoKeySheet: View {
    @ObservedObject var model: DeployModel
    let link: AppLink
    @State private var picking = false
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(link.at.isEmpty ? "This link is from a bia whose key is not on this phone." : "This link is from the bia at \(link.at.joined(separator: ", ")), and its key is not on this phone.")
                }
                if let p = model.keyProblem {
                    Section("The key this link brought") {
                        Text(p).foregroundStyle(.red)
                        Text("Ask bia in the chat for a new link: its key lasts 10 minutes and works once.")
                    }
                }
                Section("Or with a .biaconfig") {
                    Text("Ask bia in any chat: “send me a .biaconfig for my phone”. It sends a file and tells the password another way.")
                    Text("Open the file with the bia app (tap it in the chat) and enter the password. Then this link opens by itself.")
                        .foregroundStyle(.secondary)
                    Button("Open a .biaconfig…") { picking = true }
                        .fileImporter(isPresented: $picking, allowedContentTypes: [UTType(exportedAs: "biadeploy.config")]) { result in
                            if case .success(let url) = result { model.offerConfig(url) }
                        }
                }
            }
            .navigationTitle("No key for this bia")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close") { model.closeNoKey() } } }
        }
    }
}

/// This phone's key as a .biaconfig, for the share sheet.
struct ExportSheet: View {
    let folder: String
    let host: String
    let name: String
    @Environment(\.dismiss) private var dismiss
    @State private var password = (try? biaconfigPassword()) ?? ""
    @State private var problem: String?
    @State private var busy = false
    @State private var file: URL?
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("Whoever opens it holds this same certificate, with everything it can do on this bia. Revoking it later ends it for this phone too.")
                    TextField("Password", text: $password).textInputAutocapitalization(.never).autocorrectionDisabled()
                        .onChange(of: password) { _ in file = nil }
                    Button("Copy the password") { UIPasteboard.general.string = password }
                    Text("Send the password another way, never next to the file.").font(.footnote).foregroundStyle(.secondary)
                    if let problem { Text(problem).foregroundStyle(.red) }
                }
                Section {
                    if let file {
                        ShareLink(item: file, subject: Text("bia \(host)")) { Label("Share \(file.lastPathComponent)", systemImage: "square.and.arrow.up") }
                    } else {
                        Button(busy ? "Locking…" : "Make the file") { make() }.disabled(busy || password.isEmpty)
                    }
                }
            }
            .navigationTitle("Share as .biaconfig")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } } }
        }
        .onDisappear { if let file { try? FileManager.default.removeItem(at: file) } }
    }

    private func make() {
        busy = true
        problem = nil
        let out = FileManager.default.temporaryDirectory.appendingPathComponent("biaconfig-\(UUID().uuidString)", isDirectory: true).path
        let (f, h, n, p) = (folder, host, name, password)
        Task {
            defer { busy = false }
            do {
                let path = try await Task.detached { try biaconfigExport(folder: f, host: h, name: n, password: p, outDir: out) }.value
                file = URL(fileURLWithPath: path)
            } catch DeployError.Failed(let reason) {
                problem = reason
            } catch {
                problem = error.localizedDescription
            }
        }
    }
}
