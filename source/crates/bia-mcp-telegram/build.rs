fn main() {
    // ntgcalls-sys links the native library. For guest builds we always want the
    // runtime search path to be the speech tree shipped in the Debian rootfs.
    println!("cargo:rerun-if-env-changed=NTGCALLS_LIB_DIR");
    println!("cargo:rerun-if-env-changed=NTGCALLS_DYLIB");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/local/lib/bia/speech/lib");
}
