bia home (/home/bia). The agent works here.

  bia.toml  configuration: model, sampling, system prompt, web app.
  model/    local *.gguf models (split parts allowed). bia.toml picks which one.
            Not packed into the image: bia-deploy puts a Hugging Face GGUF on the stick.
  files/    the agent's own files: notes, memory, schedules, inbox, state.
  MCPs/     tool servers. One folder per MCP with mcp.json + bin/<arch>/<binary>.

The small FAT32 partition BIABOOT (mounted at /boot) holds shim, GRUB, the
kernel, the initramfs and the rescue image used for OS updates.
