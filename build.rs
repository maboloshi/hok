fn main() {
    let now = jiff::Zoned::now();
    let date = now.strftime("%Y-%m-%d %H:%M:%S %z").to_string();
    println!("cargo:rustc-env=BUILD_DATE={}", date);
}
