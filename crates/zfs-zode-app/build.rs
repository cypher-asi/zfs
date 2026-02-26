fn main() {
    // Windows' default main-thread stack is 1 MB which is too small for the
    // deep egui render pipeline combined with post-quantum crypto types
    // (ML-DSA-65 / ML-KEM-768 keys are multi-KB each).  Request 8 MB.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
}
