package biadeploy.android

import uniffi.bia_deploy_mobile.Done
import uniffi.bia_deploy_mobile.Facts
import uniffi.bia_deploy_mobile.Plan
import uniffi.bia_deploy_mobile.PlanRow
import uniffi.bia_deploy_mobile.SshKey
import java.io.File

/**
 * Debug builds only (store screenshots, play-assets.sh): `am start … --es demo <scene>` fills
 * the wizard with made-up data and no network. Scenes: brain, login, newkey, confirm,
 * running, installed, keys. Addresses are from the documentation ranges (203.0.113.0/24).
 */
fun DeployModel.demo(scene: String, filesDir: File) {
    val host = "203.0.113.10"
    provider = "openrouter"
    apiKey = "sk-or-v1-" + "0".repeat(64)
    model = "deepseek/deepseek-v4.1-flash"
    models.clear()
    hello = "Hi! I'm awake and ready. Show me my new home and I'll move in."
    markWoken()
    if (scene == "brain") return

    page = Page.Server
    this.host = host
    user = "root"
    how = How.Password
    password = "demo-password"
    if (scene == "login") return
    if (scene == "newkey") {
        this.host = ""
        how = How.NewKey
        generated = SshKey(public = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGs7Vb3q9wJ2dYxk4mR8pT1nZcLh0eUo5fAiQ6bXy2Wz bia-deploy", path = "")
        return
    }

    facts = Facts(hostname = "vps-frankfurt-1", os = "Ubuntu 24.04 LTS", arch = "x86_64", ramMib = 8192u, rootLogin = true)
    plan = Plan(disk = "/dev/vda", land = false, rows = listOf(
        PlanRow(what = "mode", value = "new agent"),
        PlanRow(what = "fmm", value = "openrouter deepseek/deepseek-v4.1-flash key sk-or-v1-…0000"),
        PlanRow(what = "lang", value = "English (en)"),
        PlanRow(what = "root certificate", value = "made here, kept on this phone"),
        PlanRow(what = "server certificate", value = "bia, localhost, $host"),
        PlanRow(what = "disk", value = "/dev/vda 160 GiB QEMU HARDDISK"),
    ))
    step = Step.Confirm
    if (scene == "confirm") return

    if (scene == "running") {
        step = Step.Running
        progress = "bia's system is downloading — 1.3 of 2.1 GB"
        fraction = 0.62f
        keySaved = true
        return
    }

    // a few bias whose keys this phone keeps
    val servers = File(filesDir, "servers")
    listOf(host, "198.51.100.24", "203.0.113.77").forEachIndexed { i, h ->
        File(servers, h).apply {
            mkdirs()
            File(this, "bia-root.p12").writeText("demo")
            File(this, "ca.crt").writeText("demo")
            setLastModified(System.currentTimeMillis() - i * 86_400_000L)
            File(this, "bia-root.p12").setLastModified(System.currentTimeMillis() - i * 86_400_000L)
        }
    }
    if (scene == "keys") {
        page = Page.Brain
        openKeys()
        return
    }
    keySaved = true
    rebooting = true
    done = Done(folder = File(servers, host).absolutePath, caPem = "", caSha256 = "", agentName = "bia-3f9c21")
    step = Step.Installed
    up = true
}
