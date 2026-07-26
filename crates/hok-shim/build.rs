fn main() {
    // Resolve duplicate memcpy/memset/memcmp symbols with CRT in debug builds
    println!("cargo:rustc-link-arg=/FORCE:MULTIPLE");
}
