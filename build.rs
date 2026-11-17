fn main() {
    // Embed the MegaTemp logo as the Windows icon of the unified executable.
    #[cfg(windows)]
    {
        let assets = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("assets");
        let _ = embed_resource::compile_for(
            "assets/megatemp.rc",
            &["megatemp"],
            embed_resource::ParamsIncludeDirs(vec![assets]),
        );
    }
}
