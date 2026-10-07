//! Windows: embed the app icon (assets/labelwerk.rc) into Labelwerk.exe.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/labelwerk.rc");
        println!("cargo:rerun-if-changed=assets/Labelwerk.ico");
        embed_resource::compile("assets/labelwerk.rc", embed_resource::ParamsIncludeDirs(["assets"]))
            .manifest_optional()
            .expect("compiling the Windows resources");
    }
}
