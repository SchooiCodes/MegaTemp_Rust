//! Embed the MegaTemp icon into the Windows executables.
fn main() {
    #[cfg(target_os = "windows")]
    embed_resource::compile("assets/megatemp.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
