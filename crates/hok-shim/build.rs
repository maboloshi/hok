fn main() {
    // Our memcpy/memset/memcmp implementations (named differently to avoid
    // duplicate symbol conflict with CRT). /ALTERNATENAME redirects the
    // standard names to our implementations at link time.
    println!("cargo:rustc-link-arg=/ALTERNATENAME:memcpy=shim_memcpy");
    println!("cargo:rustc-link-arg=/ALTERNATENAME:memset=shim_memset");
    println!("cargo:rustc-link-arg=/ALTERNATENAME:memcmp=shim_memcmp");
}
