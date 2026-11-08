fn main() {
    println!("cargo:rerun-if-changed=assets/icons/jimoto.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/jimoto.ico")
            .set("ProductName", "Talk風Live翻訳")
            .set("FileDescription", "Talk風Live翻訳 - translated by PLaMo")
            .compile()
            .expect("Could not embed the application icon; install the Windows SDK.");
    }
}
