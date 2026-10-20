<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span>
          <NuxtLink to="/download">Get bia</NuxtLink><span aria-hidden="true">/</span><span>Servers and virtual machines</span>
        </nav>
        <span class="eyebrow">For servers, clouds and virtual machines</span>
        <h1>bia on a server or a virtual machine</h1>
        <p class="section-lead">
          A rented server, a machine in a rack or a virtual machine can become bia too. It answers on the internet
          day and night, and you reach it from anywhere through its private web app. Pick the way that fits:
        </p>
        <div class="grid grid--3 ways">
          <a v-for="w in ways" :key="w.id" class="card way" :href="`#${w.id}`">
            <span class="way__tag">{{ w.tag }}</span>
            <h3>{{ w.title }}</h3>
            <p>{{ w.text }}</p>
          </a>
        </div>
      </div>
    </section>

    <!-- ───────────── takeover ───────────── -->
    <section id="takeover" class="section anchor">
      <div class="container">
        <span class="eyebrow">Way 1 · one command</span>
        <h2 class="h2 h2--top">Turn a running Linux server into bia</h2>
        <p class="section-lead">
          Have a server with Ubuntu, Debian or another Linux, for example a VPS from a hosting provider? Log in over
          ssh and run one command. It asks a few questions, checks everything, and only then replaces the system
          with bia. The server keeps its network settings, so it comes back at the same address.
        </p>
        <pre class="screen screen--cmd">curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash</pre>

        <h3 class="h3">What happens</h3>
        <ol class="flow">
          <li v-for="(s, i) in takeoverSteps" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
              <pre v-if="s.screen" class="screen">{{ s.screen }}</pre>
            </div>
          </li>
        </ol>

        <h3 class="h3">What the server needs</h3>
        <ul class="list-check">
          <li>
            <strong>Linux on Intel or AMD (x86_64) or ARM (arm64)</strong>, with root access. Modern (UEFI) and
            classic (BIOS) start-up both work on Intel and AMD; ARM servers need UEFI. Classic machines need GRUB 2,
            which almost every Linux uses.
          </li>
          <li>
            <strong>About 4 GB of memory</strong> (3 GB with classic start-up): bia is loaded into memory before
            the disk is written. The command measures this first and stops with a clear message if it is not enough.
          </li>
          <li><strong>A disk of at least 8 GB</strong>, and about 1.5 GB free on it for the download. The whole disk is replaced.</li>
          <li><strong>Internet access</strong>, and an API key for bia’s cloud brain (not needed in land mode).</li>
          <li>
            <strong>Open ports:</strong> 443 for bia’s web app, 8443 for a few minutes for the certificate page,
            and 4791 in land mode. Check the provider’s firewall too.
          </li>
          <li>
            <strong>The provider’s console, just in case.</strong> You will not need it if all goes well, but it
            is your way back in if the network does not come up.
          </li>
        </ul>
        <p class="aside">
          Before you confirm, nothing on the server changes. If you change your mind afterwards but before the
          restart, the command tells you which two things to remove.
        </p>

        <h3 class="h3">Answers from a file or a link</h3>
        <p>
          For several servers, or to run the command without questions, put the answers into a small JSON file and
          pass it with <code>--config</code>: a file on the server or an <code>https://</code> link. Give as many as
          you like; later ones win, and each may hold only part of the answers, for example only the key. Whatever is
          still missing is asked for, or, with <code>--yes</code>, left empty when it is optional.
        </p>
        <div class="grid grid--2 cfg">
          <div>
            <pre class="screen">{{ configExample }}</pre>
          </div>
          <div>
            <pre class="screen">{{ configCommand }}</pre>
            <ul class="cfg__list">
              <li><code>setup</code>: instructions for bia, read once when it first wakes up.</li>
              <li><code>pki.names</code> and <code>pki.ips</code>: names and addresses for bia’s web certificate, for example the public address behind NAT.</li>
              <li><code>--config-sha256</code> pins a link to exactly that file.</li>
              <li><code>--dump-config</code> shows the plan without changing anything; <code>bia-flash takeover --help</code> lists every option.</li>
            </ul>
          </div>
        </div>

        <h3 class="h3">Or: a land, waiting for your bia</h3>
        <p>
          Already have a bia, with its memory, accounts and skills? Then the server does not need its own setup.
          Add <code>--land</code>: the server becomes an empty bia that waits for one of yours to fly its mind there
          over the internet. No key, language or certificate is asked for; the arriving bia brings all of that.
        </p>
        <ol class="flow">
          <li v-for="(s, i) in landSteps" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
              <pre v-if="s.screen" class="screen">{{ s.screen }}</pre>
            </div>
          </li>
        </ol>
        <p class="aside">
          The land secret stays on the server through restarts and updates while it waits, and is deleted as soon as
          a bia has landed. Make it with enough letters: at least 26, which is what bia makes for you.
        </p>
      </div>
    </section>

    <!-- ───────────── agent does it ───────────── -->
    <section id="by-bia" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">Way 2 · in one sentence</span>
        <h2 class="h2 h2--top">Let your bia set up the server itself</h2>
        <p class="section-lead">
          bia knows how to do all of the above on its own. Give it ssh access to a server, ask, and it makes the land
          secret, takes the server over in land mode, waits for it to restart and flies there. You only watch.
        </p>
        <div class="grid grid--2">
          <div class="chat card">
            <p v-for="(l, i) in agentChat" :key="i" class="chat__line" :class="`chat__line--${l.from}`">
              <span v-if="l.from !== 'note'" class="chat__who">{{ l.from === 'you' ? 'You' : 'bia' }}</span>
              {{ l.text }}
            </p>
          </div>
          <ul class="list-check">
            <li>
              <strong>ssh access</strong>: a key bia can use, or a password you tell it. bia has ssh installed.
            </li>
            <li>
              <strong>Another bia’s body.</strong> The server starts as a copy of the bia that flew there. If only one
              of them should answer your messages, tell the other one to stay quiet, or switch it off.
            </li>
            <li>
              <strong>The same rules</strong> as above: about 4 GB of memory, port 4791 open, internet access.
            </li>
            <li>
              <strong>Many servers.</strong> Give bia several addresses, and it prepares and flies to each in turn,
              with a separate land secret for every server.
            </li>
          </ul>
        </div>
      </div>
    </section>

    <!-- ───────────── installer ───────────── -->
    <section id="installer" class="section anchor">
      <div class="container">
        <span class="eyebrow">Way 3 · from an installer image</span>
        <h2 class="h2 h2--top">Install on an empty machine</h2>
        <p class="section-lead">
          For a new virtual machine, a server in a rack, or a computer with no system on it yet: the land installer
          puts bia onto the disk and then waits. A bia you already have flies over the network, and the new machine
          wakes up as its copy, with the same settings, accounts and memory.
        </p>

        <h3 class="h3">1. Download the installer</h3>
        <div class="grid grid--2">
          <div v-for="a in arches" :key="a.id" class="card dl">
            <h3>{{ a.label }}</h3>
            <p class="dl__note">{{ a.note }}</p>
            <a
              class="btn btn--primary dl__btn"
              :class="{ 'is-busy': loading }"
              :href="landDownloads[a.id].url"
              :aria-busy="loading"
            >
              {{ buttonText(a.id) }}
            </a>
            <p class="dl__meta">
              <code>{{ LAND_ASSETS[a.id] }}</code>
              <template v-if="landDownloads[a.id].available">
                · version {{ landDownloads[a.id].version }}
                <template v-if="landDownloads[a.id].size"> · {{ mib(landDownloads[a.id].size) }}</template>
              </template>
            </p>
          </div>
        </div>
        <div class="note-box tip">
          <p>
            <strong>The download is compressed</strong> (<code>.iso.xz</code>), because the installer is larger
            than GitHub allows for a single file. Unpack it once to get the <code>.iso</code>:
          </p>
          <ul>
            <li><strong>Mac:</strong> double-click the file, or run <code>xz -dk bia-land-x86_64.iso.xz</code> in Terminal.</li>
            <li><strong>Linux:</strong> <code>xz -dk bia-land-x86_64.iso.xz</code></li>
            <li><strong>Windows:</strong> open it with <a href="https://www.7-zip.org/">7-Zip</a> and extract.</li>
            <li>
              <strong>Writing a USB stick?</strong> No need to unpack: balenaEtcher and Raspberry Pi Imager write the
              <code>.iso.xz</code> directly.
            </li>
          </ul>
        </div>

        <h3 class="h3">2. What the machine needs</h3>
        <ul class="list-check">
          <li>
            <strong>Modern (UEFI) start-up.</strong> The installer does not start the classic (BIOS) way; for a
            classic machine that already runs Linux, use <a href="#takeover">the one command</a> instead. Pick the
            download that matches the machine: Intel or AMD, or ARM.
          </li>
          <li>
            <strong>A disk of at least 8 GB</strong>, 32 GB or more recommended for files and programs. The installer
            erases the whole disk you choose.
          </li>
          <li><strong>Memory and processor:</strong> we test with 4 GB of memory and 4 cores.</li>
          <li>
            <strong>A console for the first minutes</strong>: the machine’s screen, the console of your virtual
            machine, or a serial console. You answer a few questions there.
          </li>
          <li>
            <strong>Internet access.</strong> After installing, the machine waits until it can reach the internet,
            because the arriving bia reinstalls its extra programs.
          </li>
          <li>
            <strong>A bia that can reach it.</strong> On the same network it is found automatically. Anywhere else,
            the server must accept connections on TCP port 4791 from that bia, at least during the move.
          </li>
        </ul>
        <p class="aside">
          The server gets no AI model of its own: it thinks with the cloud brain of the bia that flies to it.
        </p>

        <h3 class="h3">3. Install, then let bia fly in</h3>
        <ol class="flow">
          <li v-for="(s, i) in isoSteps" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
              <pre v-if="s.screen" class="screen">{{ s.screen }}</pre>
            </div>
          </li>
        </ol>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container">
        <h2 class="h2">Virtual machine settings</h2>
        <p class="section-lead">
          Any hypervisor works if the virtual machine starts the UEFI way. Attach the unpacked <code>.iso</code> as
          its CD or DVD drive and give it an empty disk. A virtual machine that already runs Linux can also use
          <a href="#takeover">the one command</a>.
        </p>
        <div class="grid grid--3">
          <div v-for="h in hypervisors" :key="h.name" class="card">
            <h3>{{ h.name }}</h3>
            <p>{{ h.text }}</p>
          </div>
        </div>
        <p class="aside">
          Our automated tests run the installer and the takeover in QEMU, on Intel and ARM, with modern and classic
          start-up. The other hypervisors follow the same standards, but we do not test them ourselves.
        </p>

        <h2 class="h2">Physical servers and clouds</h2>
        <div class="grid grid--3">
          <div v-for="p in places" :key="p.name" class="card">
            <h3>{{ p.name }}</h3>
            <p>{{ p.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="h2">Good to know</h2>
        <ul class="list-check">
          <li v-for="g in goodToKnow" :key="g.title">
            <strong>{{ g.title }}</strong> {{ g.text }}
          </li>
        </ul>
      </div>
    </section>

    <CtaBand title="Prefer a USB stick?" text="For a computer on your desk, the writer makes a stick in a few minutes." />
  </main>
</template>

<script setup lang="ts">
import { LAND_ASSETS, type LandArch } from '~/composables/useLatestRelease'

useSeoMeta({
  title: 'bia on servers, VPS and virtual machines',
  description:
    'Turn a running Linux server or VPS into bia with one command, let your bia set up a server over ssh and fly there, or install from the land installer on an empty machine.',
})

const ways = [
  {
    id: 'takeover',
    tag: 'Running Linux',
    title: 'One command',
    text: 'Run it on a rented server or any Linux machine. It becomes a new bia, or a land that waits for yours.',
  },
  {
    id: 'by-bia',
    tag: 'You have a bia',
    title: 'Ask your bia',
    text: 'Give it ssh access to a server. It sets the server up by itself and flies there.',
  },
  {
    id: 'installer',
    tag: 'Empty machine',
    title: 'Installer image',
    text: 'Start a new virtual machine or a bare server from the ISO, then let a bia fly in.',
  },
]

const takeoverSteps = [
  {
    title: 'Answer a few questions',
    text: 'The API key for bia’s cloud brain, your language, and optional setup instructions for bia. The command finds the disk, the network (with each card’s hardware address) and how the server starts, and wakes bia’s brain once to check the key.',
    screen: '== this machine: x86_64, UEFI, 4096 MiB RAM\nAPI key for bia\'s model (OpenRouter, OpenAI, Anthropic, xAI, DeepSeek, Qwen): sk-…\nOwner\'s language (code or name) [en]:\nSetup instructions for bia (optional): bia\'s model reads them once at its first boot\n> Connect to the bia at https://203.0.113.5 and set up trust with it.\n>\nPublic address or DNS name of this machine, if it is behind NAT or has a name (optional):',
  },
  {
    title: 'Check the plan and confirm',
    text: 'You see everything that will happen, and where each value came from. To go on, type the name of the disk that will be replaced. Until then, nothing has changed.',
  },
  {
    title: 'Download your personal key',
    text: 'The command makes bia’s certificates right there and opens a one-time page on port 8443 with your personal key (root’s certificate, password 12345678). Download it and add it to your browser. Without it you cannot open bia’s web app later. You can also copy /root/bia-root.p12 over ssh.',
  },
  {
    title: 'The server restarts into bia',
    text: 'bia writes itself onto the disk from memory, checks every byte, and starts with your key, language and network. This takes a few minutes; the provider’s console shows the progress.',
  },
  {
    title: 'Open bia',
    text: 'Go to https://<the server’s address>/ and choose your certificate. bia greets you there, reads your setup instructions once, and gets to work. Telegram, WhatsApp and everything else are set up from the chat, like on any bia.',
  },
]

const landSteps = [
  {
    title: 'Get a land secret',
    text: 'Ask your bia for a new land secret for that server, or run bia-flash land-secret. It is a long code that only you and your bia know.',
  },
  {
    title: 'Take the server over in land mode',
    text: 'The secret goes along. The server writes it down for when bia arrives, and asks nothing else.',
    screen: 'curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh \\\n  | sudo bash -s -- --land --land-secret 4XKP-9W2M-…',
  },
  {
    title: 'Tell your bia to fly',
    text: '“Fly to 203.0.113.7 with land secret 4XKP-9W2M-…” Both sides prove they know the secret, so nobody has to type a code at the server, and bia travels encrypted. The server wakes up as your bia, with everything it knows.',
  },
]

const agentChat = [
  { from: 'you', text: 'Here is a new server: root@203.0.113.7, the ssh key is in your files. Make it a land and fly there.' },
  { from: 'bia', text: 'Made a land secret for 203.0.113.7. The disk is /dev/vda, 40 GB, 4 GB of memory. Taking it over in land mode now…' },
  { from: 'note', text: 'Five minutes later.' },
  { from: 'bia', text: 'The server restarted and is waiting. Flying… Landed. My copy on 203.0.113.7 is awake and has my memory. Should it answer Telegram, or should I?' },
]

const configExample = `{
  "fmm": { "key": "sk-…" },
  "lang": "de",
  "setup": "Connect to the bia at https://203.0.113.5 and set up trust with it.",
  "pki": { "names": ["bia.example.com"] }
}`

const configCommand = `curl -fsSL …/takeover.sh | sudo bash -s -- \\
  --config https://example.com/bia.json \\
  --wipe /dev/vda --yes --reboot`

const arches: { id: LandArch; label: string; note: string }[] = [
  { id: 'x86_64', label: 'Intel and AMD (x86_64)', note: 'Most servers, cloud machines and virtual machines.' },
  { id: 'arm64', label: 'ARM (arm64)', note: 'ARM servers and cloud machines, and virtual machines on Apple Silicon Macs.' },
]

const isoSteps = [
  {
    title: 'Start the machine from the installer',
    text: 'Attach the ISO as the virtual machine’s CD drive, mount it through the server’s management console, or start from a USB stick. The installer explains what it is about to do.',
  },
  {
    title: 'Set up the network',
    text: 'Pick the network interface if there are several, then answer whether to use DHCP. Without DHCP, type the address with its prefix, the gateway and the DNS servers.',
    screen: 'Use DHCP? [yes/no]\n> no\nAddress with prefix (for example 192.168.1.20/24)\n> 10.0.0.20/24\nDefault gateway\n> 10.0.0.1\nDNS servers [1.1.1.1 1.0.0.1]\n>',
  },
  {
    title: 'Choose the disk',
    text: 'The installer lists the disks it can use and hides the one it started from. Type the name of the disk to erase, for example sda, vda or nvme0n1. Copying the system takes a few minutes.',
  },
  {
    title: 'Eject the ISO and restart',
    text: 'Remove the ISO from the drive and press Enter. The machine starts from its disk, brings up the network, waits for the internet and then shows that it is waiting for another bia to fly here.',
  },
  {
    title: 'Ask your bia to fly',
    text: 'Tell an existing bia, for example: “Fly to the server at 10.0.0.20”. On the same network, “Fly to the waiting computer” is enough. Both screens show the same six-digit code: type it on the server’s console and press Enter, and tell your bia that the codes match.',
  },
  {
    title: 'The server wakes up as bia',
    text: 'The server restarts with the settings, accounts, chats and memory of the bia that flew in, and reinstalls its extra programs for its own processor.',
  },
]

const hypervisors = [
  {
    name: 'Proxmox VE',
    text: 'BIOS: OVMF (UEFI), with an EFI disk. Machine type q35. VirtIO disks and network work. Secure Boot with the pre-enrolled keys can stay on for Intel and AMD.',
  },
  {
    name: 'VMware',
    text: 'ESXi, Workstation and Fusion: set the firmware to UEFI in the virtual machine’s boot options.',
  },
  {
    name: 'VirtualBox',
    text: 'Settings → System → turn on “Enable EFI”.',
  },
  {
    name: 'Hyper-V',
    text: 'Create a Generation 2 machine. Set the Secure Boot template to “Microsoft UEFI Certificate Authority”, or turn Secure Boot off.',
  },
  {
    name: 'QEMU, KVM and libvirt',
    text: 'Start with UEFI firmware: OVMF for Intel and AMD, the edk2 AArch64 firmware for ARM. With libvirt: virt-install --boot uefi.',
  },
  {
    name: 'Apple Silicon Mac',
    text: 'Use the ARM installer in an ARM Linux virtual machine, for example in UTM, Parallels or VMware Fusion.',
  },
]

const places = [
  {
    name: 'Rented server (VPS)',
    text: 'Order it with Ubuntu or Debian, log in over ssh and run the one command. Most providers have no screen at all; bia gets a virtual one, so the web app’s screen view still works.',
  },
  {
    name: 'Server with a management console',
    text: 'Run the one command if it already has Linux, or mount the unpacked ISO as virtual media through iDRAC, iLO, IPMI or a similar console and answer the installer in its remote screen.',
  },
  {
    name: 'Cloud machine with your own ISO',
    text: 'Any provider that lets you start your own ISO and gives you a console (VNC or serial) works with the installer. On ARM machines, switch Secure Boot off if it does not start.',
  },
]

const goodToKnow = [
  {
    title: 'The network comes along.',
    text: 'The one command copies the server’s addresses, gateway and DNS, tied to each network card’s hardware address, including static addresses and gateways outside the server’s own network, as many providers use. If they do not work after the restart, bia falls back to DHCP after two minutes.',
  },
  {
    title: 'Your key is the only way in.',
    text: 'bia has no password login. Its web app opens only with a certificate that bia issued. Keep root’s certificate safe, and ask bia for more keys for other people later.',
  },
  {
    title: 'Two bias, one handshake.',
    text: 'Setup instructions can tell a new bia to connect to another one. It gives the other bia a certificate and asks for one back; when the other bia’s owner approves the request (which shows the address it came from), both can work with each other.',
  },
  {
    title: 'Flying copies.',
    text: 'The bia that flies keeps running, and the server starts as its copy. If only one of them should answer your messages, switch the other one off.',
  },
  {
    title: 'Many servers, one setup.',
    text: 'Prepare one bia the way every server should be, then fly it to each waiting machine in turn, or let it prepare the servers itself.',
  },
  {
    title: 'A waiting server keeps waiting,',
    text: 'even after a restart or an update, until a bia flies in.',
  },
]

const { landDownloads, loading, fetched } = useLatestRelease()

function buttonText(id: LandArch): string {
  if (loading.value) return 'Checking latest…'
  if (fetched.value && !landDownloads.value[id].available) return 'See releases'
  return 'Download installer'
}

function mib(bytes: number): string {
  return bytes >= 1 << 30 ? `${(bytes / (1 << 30)).toFixed(1)} GB` : `${Math.round(bytes / (1 << 20))} MB`
}
</script>

<style scoped>
.h2 {
  font-size: clamp(1.4rem, 3vw, 1.9rem);
  margin: 2.5rem 0 1rem;
}
.h2:first-child, .h2--top { margin-top: 0.3rem; }
.h3 { font-size: 1.25rem; margin: 2rem 0 0.75rem; }
.anchor { scroll-margin-top: 4.5rem; }
.ways { margin-top: 1.5rem; }
.way { display: block; color: inherit; text-decoration: none; transition: transform 0.15s ease, border-color 0.15s ease; }
.way:hover { transform: translateY(-2px); border-color: var(--accent); }
.way h3 { font-size: 1.25rem; margin: 0.3rem 0; }
.way p { margin: 0; color: var(--ink-soft); }
.way__tag { font-size: 0.8rem; font-weight: 700; color: var(--accent); text-transform: uppercase; letter-spacing: 0.04em; }
.dl h3 { font-size: 1.3rem; margin-bottom: 0.2rem; }
.dl__note { color: var(--mute); }
.dl { display: flex; flex-direction: column; }
.dl__btn { width: 100%; margin-top: auto; }
.dl__btn.is-busy { opacity: 0.7; pointer-events: none; }
.dl__meta { margin: 0.5rem 0 0; font-size: 0.85rem; color: var(--mute); text-align: center; overflow-wrap: anywhere; }
.tip { margin-top: 1.25rem; }
.tip p { margin: 0; }
.tip ul { margin: 0.75rem 0 0; padding-left: 1.2rem; }
.tip li + li { margin-top: 0.35rem; }
.aside { margin: 1rem 0 0; color: var(--mute); }
.cfg { align-items: start; }
.cfg > div, .grid > * { min-width: 0; }
code { overflow-wrap: anywhere; }
.cfg .screen { margin-top: 0; }
.cfg__list { margin: 0.75rem 0 0; padding-left: 1.2rem; color: var(--ink-soft); }
.cfg__list li + li { margin-top: 0.35rem; }
.chat { display: grid; gap: 0.6rem; align-content: start; }
.chat__line { margin: 0; padding: 0.6rem 0.8rem; border-radius: 0.75rem; background: var(--bg, #fff); border: 1px solid var(--line); }
.chat__line--you { background: color-mix(in srgb, var(--accent) 10%, transparent); }
.chat__line--note { background: none; border: none; text-align: center; font-size: 0.85rem; color: var(--mute); font-style: italic; }
.chat__who { display: block; font-size: 0.75rem; font-weight: 700; color: var(--mute); margin-bottom: 0.15rem; }
.flow {
  list-style: none;
  margin: 1rem 0 0;
  padding: 0;
  display: grid;
  gap: 1.25rem;
}
.flow__item {
  display: grid;
  grid-template-columns: 3.2rem 1fr;
  gap: 1rem;
  align-items: start;
  padding: 1.25rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  min-width: 0;
}
.flow__item > div { min-width: 0; }
.flow__item h3 { font-size: 1.2rem; margin-bottom: 0.3rem; }
.flow__item p { margin: 0; color: var(--ink-soft); }
.flow__n {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 3rem;
  height: 3rem;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 1.3rem;
}
.screen {
  margin: 0.75rem 0 0;
  padding: 0.75rem 1rem;
  background: #1d2126;
  color: #e8ece4;
  border-radius: 0.5rem;
  font-size: 0.85rem;
  line-height: 1.5;
  overflow-x: auto;
}
.screen--cmd { margin: 1.25rem 0 0; font-size: 0.9rem; }
@media (max-width: 560px) {
  .flow__item { grid-template-columns: 1fr; }
}
</style>
