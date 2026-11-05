fn main() {
    println!("cargo:rerun-if-changed=assets/icons/jimoto.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/jimoto.ico")
            .set("ProductName", "地元翻訳くん")
            .set("FileDescription", "地元翻訳くん - translated by PLaMo")
            .compile()
            .expect("Could not embed the application icon; install the Windows SDK.");
    }
}
