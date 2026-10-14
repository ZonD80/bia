# bia — boot straight into an agent.
# Target: arm64 | x86_64
ARCH ?= arm64
PLATFORM_arm64 = linux/arm64
PLATFORM_x86_64 = linux/amd64
PLATFORM = $(PLATFORM_$(ARCH))
DOCKER_RUN = docker run --rm --privileged --platform $(PLATFORM) -v $(CURDIR):/work -v bia-cache-$(ARCH):/cache bia-build:$(ARCH)

NET ?= bridge
INTERNAL ?= 1
# QEMU flash image is 16 GiB. ESP is 256 MiB; the rest is one ext4, grown to fill it.
IMAGE_MB ?= 16384
ROOT_MB ?= 7100
DISK ?=
YES ?=
ERASE ?=
# Local model for the stick from Hugging Face: MODEL=repo or MODEL=repo:file.gguf
# (gated repos: export HF_TOKEN). The image itself never contains a model.
MODEL ?=
# Optional path to local FMM JSON (default: ./fmm.json or source/flash/files/state/fmm.json). Never commit keys.
FMM ?=

.PHONY: builder build rust mcps rootfs local-files image flash flasher iso run persist-root run-serial clean patch-telegram patch-whatsapp patch-mcp website-deploy website-dev run-update-prep run-update run-update-disk run-update-status update-e2e land-e2e peer-e2e bios-e2e bios-update-e2e

website-deploy:     ## static Nuxt site → Cloudflare Pages (production branch)
	./website/deploy.sh

website-dev:        ## Nuxt website local dev server
	./website/dev.sh

builder:            ## Debian build container (glibc dynamic agent + MCPs)
	docker build --platform $(PLATFORM) -t bia-build:$(ARCH) builders

build:              ## agent + MCPs + debian rootfs for $(ARCH)
	$(DOCKER_RUN) env ROOT_MB=$(ROOT_MB) /work/builders/build.sh $(ARCH) all
	tools/local-files.sh $(ARCH)

rust:
	$(DOCKER_RUN) /work/builders/build.sh $(ARCH) rust

mcps:
	$(DOCKER_RUN) /work/builders/build.sh $(ARCH) mcps

rootfs:
	$(DOCKER_RUN) env ROOT_MB=$(ROOT_MB) /work/builders/build.sh $(ARCH) rootfs
	tools/local-files.sh $(ARCH)

local-files:        ## dist/$(ARCH)/local-files.json for the bia-flash GUI (after a build)
	tools/local-files.sh $(ARCH)

image:              ## dist/bia.img (QEMU flash image) from dist/$(ARCH)/root.img (macOS host)
	ARCH=$(ARCH) IMAGE_MB=$(IMAGE_MB) ROOT_MB=$(ROOT_MB) tools/mkimage.sh

flash:              ## write a USB stick (x86_64 unless ARCH= is set); uses bia-flash for new sticks
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),x86_64) ROOT_MB=$(ROOT_MB) DISK=$(DISK) YES=$(YES) ERASE=$(ERASE) MODEL="$(MODEL)" FMM=$(FMM) tools/flash.sh

# NOTARIZE=0 make flasher  — codesign only (macOS). Default notarizes when .env has Apple creds.
NOTARIZE ?= 1
flasher:            ## host bia-flash (+ GUI); macOS: notarized .app/DMG + CLI zip
	@if [ "$$(uname -s)" = Darwin ]; then \
	  if [ "$(NOTARIZE)" = 0 ]; then \
	    installers/macos/release.sh --no-notarize; \
	  else \
	    installers/macos/release.sh; \
	  fi; \
	else \
	  tools/build-flasher.sh gui; \
	fi

iso:                ## land ISO, no model (x86_64 unless ARCH= is set)
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),x86_64) tools/mkiso.sh

persist-root:        ## rebuild root.img and write it over BIAROOT in the existing image
	$(MAKE) rust
	$(MAKE) mcps
	$(MAKE) rootfs
	ARCH=$(ARCH) tools/persist-root.sh

# MCP=chrome | MCP="secrets chrome" | MCP=all (default telegram for patch-telegram)
MCP ?=
patch-mcp:          ## rebuild MCP(s) and hot-patch flash/ + bia.img + internal.img
	@test -n "$(MCP)" || (echo 'usage: make patch-mcp MCP=chrome   or MCP="secrets chrome" or MCP=all' >&2; exit 1)
	ARCH=$(ARCH) tools/patch-mcp.sh $(MCP)

patch-telegram:     ## rebuild telegram MCP and hot-patch flash/ + bia.img + internal.img
	ARCH=$(ARCH) tools/patch-mcp.sh telegram

patch-whatsapp:     ## rebuild whatsapp MCP and hot-patch flash/ + bia.img + internal.img
	ARCH=$(ARCH) tools/patch-mcp.sh whatsapp

run:                ## boot the existing flash image in QEMU, no rebuild
	MODE=gui ARCH=$(ARCH) NET=$(NET) INTERNAL=$(INTERNAL) BOOT=flash tools/run-qemu.sh
run-serial:         ## same, serial console on this terminal
	MODE=serial ARCH=$(ARCH) NET=$(NET) INTERNAL=$(INTERNAL) BOOT=flash tools/run-qemu.sh

# Dual-disk update/rescue by hand (flash=bia.img + disk=internal.img). Default ARCH=x86_64.
#   make run-update-prep          labels + bump flash release + clear /bia-update
#   make run-update               boot flash (/update updates the flash from a newer disk, /rescue pushes it onto the disk)
#   make run-update-disk          boot BIADISK (/update updates it from a newer flash)
# Automated: make update-e2e [ARCH=arm64] [SCENARIOS="disk flash rescue"]  (tools/update-e2e.sh;
#   run `tools/update-e2e.sh images` once after make build)
SCENARIOS ?= disk flash rescue
update-e2e:         ## QEMU end-to-end test of carry_update / carry_rescue through the rescue image
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),arm64) tools/update-e2e.sh all $(SCENARIOS)

land-e2e:           ## QEMU end-to-end test of the land ISO install and carry fly/land (two VMs)
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),arm64) tools/land-e2e.sh all

peer-e2e:           ## QEMU end-to-end test: one agent requests a cert from another, waits for approval, then chats
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),arm64) tools/peer-e2e.sh all

bios-e2e:           ## QEMU end-to-end test on legacy BIOS (SeaBIOS, x86_64): stick boots, agent installs to disk, disk boots
	tools/bios-e2e.sh all

bios-update-e2e:    ## /update and /rescue on legacy BIOS (SeaBIOS, x86_64); run `ARCH=x86_64 tools/update-e2e.sh images patch` first
	python3 tools/bios-update-e2e.py $(SCENARIOS)
run-update-prep:
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),x86_64) tools/qemu-update-test.sh prep
run-update:
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),x86_64) MODE=$(MODE) NET=$(if $(filter command line,$(origin NET)),$(NET),user) tools/qemu-update-test.sh flash
run-update-disk:
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),x86_64) MODE=$(MODE) NET=$(if $(filter command line,$(origin NET)),$(NET),user) tools/qemu-update-test.sh disk
run-update-status:
	ARCH=$(if $(filter command line,$(origin ARCH)),$(ARCH),x86_64) tools/qemu-update-test.sh status

clean:
	rm -rf dist target
