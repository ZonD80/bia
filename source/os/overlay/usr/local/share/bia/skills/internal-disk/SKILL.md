# Disk migration

Boot does not switch disks by itself. Use the carry MCP.

carry_fold saves a gzip backup named name.unfold-<bytes>.tar.gz. It refuses when free space is below the size of /home/bia. carry_unfold puts that backup on this computer and reboots. An older backup whose name has no unfolded size still unfolds after the user confirms. Ask the user, then pass force=true only after they confirm. Do the same when free space is below that size.
carry_land waits for another computer to send its backup. Both screens show the same 6-digit code. Call carry_confirm only after the user says the codes match.
carry_discover lists computers that are waiting. carry_fly sends this backup to one of them.
carry_morph(disk=/dev/...) folds the agent from that partition onto this boot, then reboots.
carry_update starts an attended OS update (same as console /update). Not automatic at boot. On BIADISK with a newer flash stick inserted: fold this machine into /bia-update/, reboot to the flash, apply the OS offline onto BIADISK, then unfold. On flash: push this OS onto BIADISK (like carry_rescue). Insert the stick, then call carry_update (or ask root to type /update).
carry_rescue(disk=/dev/...) from a flash boot: folds the agent on BIADISK into that disk's /bia-update/, pushes this OS onto BIADISK (checksum verify; /bia-update/ is never wiped), refreshes the disk bootloader as "bia disk", then reboots so the next boot unfolds. Use when the disk will not boot. Prefer carry_update for a normal version bump. Omit disk= to find LABEL=BIADISK.
carry_to_disk(disk=/dev/sdX) copies this computer onto that whole disk, then reboots into it (BootNext "bia disk"). Does not power off.
carry_to_flash(disk=/dev/...) copies onto that USB stick. Then shut down.

When both the internal disk and a flash stick are attached, choose firmware entry "bia disk" (not Removable Media / "bia flash") for normal boots.

Always pick devices with bash (`lsblk -d -o NAME,SIZE,MODEL,TRAN,SERIAL` and `lsblk -f`), then pass `disk=` / `partition=`. Tools do not look up virtio serial tags.

If that disk already has Windows, carry_to_disk does not wipe it. Call with disk=/dev/sdX alone to list partitions and free space. Then ask the user and call again with partition=/dev/... (format that volume), shrink_from=/dev/... and size_mib=N, or size_mib=N alone (unallocated space). Boot files go on the existing EFI partition under EFI/bia; Windows stays the default BootOrder, but carry_to_disk sets a one-shot BootNext to "bia disk" and reboots. wipe=true erases the whole disk including Windows. An empty disk gets a full EFI + BIADISK layout from carry_to_disk(disk=...).

A land ISO from scripts/make_iso boots a server with no model, asks for the network, and waits. Send this computer there with carry_fly. The code is on the server screen. Confirm it here with carry_confirm.
