//! On Windows: ternarycode.exe's icon and version information (Explorer, the taskbar, Start menu
//! shortcuts, the installer). Nothing to do elsewhere.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=packaging/windows/ternarycode.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/windows/ternarycode.ico");
        res.set("ProductName", "TernaryCode");
        res.set("FileDescription", "TernaryCode");
        res.set("LegalCopyright", "MIT License");
        res.compile().expect("Windows resources (icon, version)");
    }
}
