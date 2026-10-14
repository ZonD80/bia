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
          The bia land installer puts bia onto a machine’s disk and then waits. A bia you already have flies over the
          network, and the new machine wakes up as its copy, with the same settings, accounts and memory. Use it
          wherever a USB stick is awkward: a server in a rack, a machine in a cloud, or your own virtual machines.
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="h2">1. Download the installer</h2>
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

        <h2 class="h2">2. What the machine needs</h2>
        <ul class="list-check">
          <li>
            <strong>Modern (UEFI) start-up.</strong> The installer does not start the classic (BIOS) way. Pick the
            download that matches the machine: Intel or AMD, or ARM.
          </li>
          <li>
            <strong>A disk of at least 8 GB</strong>, 32 GB or more recommended for files and programs. The installer
            erases the whole disk you choose.
          </li>
          <li><strong>Memory and processor:</strong> we test with 4 GB of memory and 4 cores.</li>
          <li>
            <strong>A console for the first minutes</strong>: the machine’s screen, the console of your virtual
            machine, or a serial console. You answer a few questions there and type one code.
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

        <h2 class="h2">3. Install, then let bia fly in</h2>
        <ol class="flow">
          <li v-for="(s, i) in steps" :key="i" class="flow__item">
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
          its CD or DVD drive and give it an empty disk.
        </p>
        <div class="grid grid--3">
          <div v-for="h in hypervisors" :key="h.name" class="card">
            <h3>{{ h.name }}</h3>
            <p>{{ h.text }}</p>
          </div>
        </div>
        <p class="aside">
          Our automated tests run the installer in QEMU. The other hypervisors follow the same UEFI standard, but we
          do not test them ourselves.
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
          <li>
            <strong>Flying copies.</strong> The bia that flies keeps running, and the server starts as its copy. If
            only one of them should answer your messages, switch the other one off.
          </li>
          <li>
            <strong>Many servers, one setup.</strong> Prepare one bia the way every server should be, then fly it to
            each waiting machine in turn.
          </li>
          <li>
            <strong>The server keeps its own address.</strong> Its network address and computer name stay the ones
            you chose during installation.
          </li>
          <li>
            <strong>A waiting server keeps waiting</strong>, even after a restart, until a bia flies in.
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
  title: 'bia on servers and virtual machines',
  description:
    'Download the bia land installer for Intel/AMD or ARM, install it on a server, cloud machine or virtual machine, and let an existing bia fly in.',
})

const arches: { id: LandArch; label: string; note: string }[] = [
  { id: 'x86_64', label: 'Intel and AMD (x86_64)', note: 'Most servers, cloud machines and virtual machines.' },
  { id: 'arm64', label: 'ARM (arm64)', note: 'ARM servers and cloud machines, and virtual machines on Apple Silicon Macs.' },
]

const steps = [
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
    name: 'Server with a management console',
    text: 'Mount the unpacked ISO as virtual media through iDRAC, iLO, IPMI or a similar console, and answer the installer in its remote screen.',
  },
  {
    name: 'Server without one',
    text: 'Write the installer to a USB stick with balenaEtcher or Raspberry Pi Imager, start the server from it, and keep a screen and keyboard attached for the first steps.',
  },
  {
    name: 'Cloud machine',
    text: 'Any provider that lets you start your own ISO and gives you a console (VNC or serial) works. On ARM machines, switch Secure Boot off if the installer does not start.',
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
.h2:first-child { margin-top: 0; }
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
@media (max-width: 560px) {
  .flow__item { grid-template-columns: 1fr; }
}
</style>
